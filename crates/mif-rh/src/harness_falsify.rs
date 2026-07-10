//! Falsification gate substrate (rht Category B, Story #287, SPEC §6b).
//!
//! Ports rht's `scripts/falsify.sh`: the deterministic, fixture-driven
//! offline gate the falsification-analyst agent writes through and the
//! smoke test exercises directly. Treats a finding as a hypothesis,
//! consults an offline evidence fixture keyed by finding `@id`, assigns an
//! ordinal verdict, and writes it back into
//! `extensions.harness.verification` — never re-grading a finding that
//! already carries a verdict from a prior round (the one-round rule).

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::error::MifRhError;
use crate::harness_project::read_json;

/// The placeholder basis recorded when no fixture entry exists for a
/// finding — it was not adversarially tested this run.
const PLACEHOLDER_BASIS: &str =
    "No disconfirming-evidence entry supplied — finding was not adversarially tested this run.";
/// The default basis for an explicit fixture verdict that supplies none.
const DEFAULT_BASIS: &str = "Adversarial queries executed; no disconfirming evidence found.";

/// The result of one [`falsify`] call.
///
/// Carries the (possibly updated) finding, and the exact operator-facing
/// log line the original script emits to stderr (either a
/// `"falsification-gate: run (...)"` or a
/// `"falsification-gate: skipped (...)"` line — callers assert on this
/// exact text to prove the gate ran exactly once per session).
#[derive(Debug, Clone)]
pub struct FalsifyResult {
    /// The finding JSON, updated with a verdict unless the one-round rule
    /// short-circuited (in which case it is returned unchanged).
    pub finding: Value,
    /// The stderr log line.
    pub log_line: String,
}

fn already_graded(finding: &Value) -> bool {
    finding
        .pointer("/extensions/harness/verification/attempted_at")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
}

/// The fixture entry for `id` — `{}` when no fixture path is given, the
/// fixture cannot be read, or it has no entry for `id`.
fn fixture_entry(fixture_path: Option<&Path>, id: &str) -> Value {
    let Some(fixture_path) = fixture_path.filter(|path| path.is_file()) else {
        return json!({});
    };
    let Ok(fixture) = read_json(fixture_path) else {
        return json!({});
    };
    fixture.get(id).cloned().unwrap_or_else(|| json!({}))
}

/// Merges `verification` into `finding`'s `extensions.harness.verification`,
/// creating `extensions`/`harness` as objects if either is absent or not
/// already an object.
fn with_verification(finding: Value, verification: Value) -> Value {
    let mut root = finding;
    let mut extensions = root
        .get("extensions")
        .cloned()
        .filter(Value::is_object)
        .and_then(|v| match v {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    let mut harness = extensions
        .get("harness")
        .cloned()
        .filter(Value::is_object)
        .and_then(|v| match v {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    harness.insert("verification".to_string(), verification);
    extensions.insert("harness".to_string(), Value::Object(harness));
    if let Value::Object(ref mut map) = root {
        map.insert("extensions".to_string(), Value::Object(extensions));
    }
    root
}

/// Runs the falsification gate over `finding_path`, consulting
/// `fixture_path` (an offline, fixture-supplied body of disconfirming
/// evidence keyed by finding `@id`) for the verdict to assign.
///
/// A finding that already carries `extensions.harness.verification.attempted_at`
/// from a prior round is returned unchanged (the one-round rule — grading it
/// again would never terminate). Otherwise an explicit fixture entry's
/// verdict is recorded as-is, defaulting `attempted_at` to `now` when the
/// fixture omits it (rather than a fixed placeholder — a fixture author
/// forgetting the field should record the real grading time, not a value
/// that reads as maximally stale to freshness projections); a finding with
/// no fixture entry is recorded as a placeholder `inconclusive` (never a
/// false `survived`) that omits `attempted_at` entirely, so a later real
/// gate run can still overwrite it. `now` is a caller-supplied parameter
/// (mirroring [`crate::resolve_membership`]'s `now`) rather than an internal
/// clock call, so tests stay deterministic and real callers pass the actual
/// wall-clock time.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `finding_path` cannot be read, and
/// [`MifRhError::Json`] if it is not valid JSON.
pub fn falsify(
    finding_path: &Path,
    fixture_path: Option<&Path>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<FalsifyResult, MifRhError> {
    let finding = read_json(finding_path)?;
    let id = finding
        .get("@id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if already_graded(&finding) {
        return Ok(FalsifyResult {
            finding,
            log_line: format!("falsification-gate: skipped (already falsified this session): {id}"),
        });
    }

    let entry = fixture_entry(fixture_path, &id);
    let explicit_verdict = entry
        .get("verdict")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty());
    let (verdict, basis, placeholder) = explicit_verdict.map_or_else(
        || {
            (
                "inconclusive".to_string(),
                PLACEHOLDER_BASIS.to_string(),
                true,
            )
        },
        |verdict| {
            (
                verdict.to_string(),
                entry
                    .get("basis")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_BASIS)
                    .to_string(),
                false,
            )
        },
    );
    let attempted_at = entry
        .get("attempted_at")
        .and_then(Value::as_str)
        .map_or_else(
            || now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            str::to_string,
        );
    let disconfirming = entry
        .get("disconfirming")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut verification = Map::new();
    verification.insert("verdict".to_string(), Value::String(verdict.clone()));
    verification.insert("verdict_basis".to_string(), Value::String(basis));
    verification.insert(
        "disconfirming_evidence".to_string(),
        Value::Array(disconfirming),
    );
    if !placeholder {
        verification.insert("attempted_at".to_string(), Value::String(attempted_at));
    }

    let updated = with_verification(finding, Value::Object(verification));
    Ok(FalsifyResult {
        finding: updated,
        log_line: format!("falsification-gate: run ({id} -> {verdict})"),
    })
}

#[cfg(test)]
mod tests {
    use super::falsify;
    use std::fs;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .to_utc()
    }

    fn write(dir: &std::path::Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_finding_with_no_fixture_gets_a_placeholder_inconclusive_without_attempted_at() {
        let dir = tempfile::tempdir().unwrap();
        let finding = write(dir.path(), "f.json", r#"{"@id": "urn:mif:f1"}"#);

        let result = falsify(&finding, None, now()).unwrap();
        assert_eq!(
            result.finding["extensions"]["harness"]["verification"]["verdict"],
            "inconclusive"
        );
        assert!(
            result.finding["extensions"]["harness"]["verification"]
                .get("attempted_at")
                .is_none()
        );
        assert_eq!(
            result.log_line,
            "falsification-gate: run (urn:mif:f1 -> inconclusive)"
        );
    }

    #[test]
    fn an_explicit_fixture_verdict_is_recorded_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let finding = write(dir.path(), "f.json", r#"{"@id": "urn:mif:f1"}"#);
        let fixture = write(
            dir.path(),
            "evidence.json",
            r#"{"urn:mif:f1": {"verdict": "falsified", "basis": "contradicted", "attempted_at": "2026-01-01T00:00:00Z", "disconfirming": ["https://example.com"]}}"#,
        );

        let result = falsify(&finding, Some(&fixture), now()).unwrap();
        let verification = &result.finding["extensions"]["harness"]["verification"];
        assert_eq!(verification["verdict"], "falsified");
        assert_eq!(verification["verdict_basis"], "contradicted");
        assert_eq!(verification["attempted_at"], "2026-01-01T00:00:00Z");
        assert_eq!(
            verification["disconfirming_evidence"][0],
            "https://example.com"
        );
    }

    #[test]
    fn an_explicit_fixture_verdict_missing_attempted_at_defaults_to_now_not_epoch_zero() {
        // Regression test for #359: a fixture that supplies a verdict but
        // omits attempted_at used to be silently stamped with a fixed
        // 1970-01-01T00:00:00Z placeholder -- semantically wrong provenance
        // that also read as maximally stale to freshness projections. It
        // must default to the injected `now` instead.
        let dir = tempfile::tempdir().unwrap();
        let finding = write(dir.path(), "f.json", r#"{"@id": "urn:mif:f1"}"#);
        let fixture = write(
            dir.path(),
            "evidence.json",
            r#"{"urn:mif:f1": {"verdict": "survived", "basis": "no contradicting evidence found"}}"#,
        );

        let result = falsify(&finding, Some(&fixture), now()).unwrap();
        let verification = &result.finding["extensions"]["harness"]["verification"];
        assert_eq!(verification["verdict"], "survived");
        assert_eq!(verification["attempted_at"], "2026-06-01T00:00:00Z");
        assert_ne!(verification["attempted_at"], "1970-01-01T00:00:00Z");
    }

    #[test]
    fn the_one_round_rule_skips_a_finding_that_already_carries_a_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let finding = write(
            dir.path(),
            "f.json",
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"verification": {
                "verdict": "survived", "verdict_basis": "x", "attempted_at": "2026-01-01T00:00:00Z"
            }}}}"#,
        );

        let before: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&finding).unwrap()).unwrap();
        let result = falsify(&finding, None, now()).unwrap();
        assert_eq!(result.finding, before);
        assert_eq!(
            result.log_line,
            "falsification-gate: skipped (already falsified this session): urn:mif:f1"
        );
    }

    #[test]
    fn preserves_existing_unrelated_fields_and_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let finding = write(
            dir.path(),
            "f.json",
            r#"{"@id": "urn:mif:f1", "title": "keep me", "extensions": {"other": "keep too"}}"#,
        );

        let result = falsify(&finding, None, now()).unwrap();
        assert_eq!(result.finding["title"], "keep me");
        assert_eq!(result.finding["extensions"]["other"], "keep too");
        assert_eq!(
            result.finding["extensions"]["harness"]["verification"]["verdict"],
            "inconclusive"
        );
    }

    #[test]
    fn errors_on_a_missing_finding_file() {
        let error = falsify(std::path::Path::new("/no/such/file.json"), None, now()).unwrap_err();
        assert!(matches!(error, super::MifRhError::Io { .. }));
    }
}
