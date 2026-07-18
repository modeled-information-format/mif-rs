//! Falsification gate substrate (rht Category B, Story #287, SPEC §6b).
//!
//! Ports rht's `scripts/falsify.sh`: the deterministic, fixture-driven
//! offline gate the falsification-analyst agent writes through and the
//! smoke test exercises directly. Treats a finding as a hypothesis,
//! consults an offline evidence fixture keyed by finding `@id`, assigns an
//! ordinal verdict, and writes it back into
//! `extensions.harness.verification` — never re-grading a finding that
//! already carries a verdict from a prior round (the one-round rule),
//! unless the caller explicitly opts into a `regate` override
//! (issue #119) for that single invocation.

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
/// log line the original script emits to stderr (a
/// `"falsification-gate: run (...)"`, `"falsification-gate: skipped (...)"`,
/// or — when the caller forces a re-grade with `regate` — a
/// `"falsification-gate: regated (...)"` line; callers assert on this exact
/// text to prove the gate ran exactly once per session, or that a `regate`
/// override genuinely re-graded an already-graded finding).
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

/// Runs the falsification gate over `finding_path`, defaulting a fixture
/// entry's missing `attempted_at` to the actual current wall-clock time.
///
/// See [`falsify_with_now`] for the full behavior; this is a thin wrapper
/// over it for callers that do not need deterministic time injection.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `finding_path` cannot be read, and
/// [`MifRhError::Json`] if it is not valid JSON.
pub fn falsify(
    finding_path: &Path,
    fixture_path: Option<&Path>,
    regate: bool,
) -> Result<FalsifyResult, MifRhError> {
    falsify_with_now(finding_path, fixture_path, chrono::Utc::now(), regate)
}

/// Runs the falsification gate over `finding_path`, consulting
/// `fixture_path` (an offline, fixture-supplied body of disconfirming
/// evidence keyed by finding `@id`) for the verdict to assign.
///
/// A finding that already carries `extensions.harness.verification.attempted_at`
/// from a prior round is returned unchanged (the one-round rule — grading it
/// again would never terminate), **unless `regate` is `true`**, in which case
/// the one-round short-circuit is bypassed for this single invocation only —
/// every other invariant (fixture-keyed verdict lookup, the
/// `extensions.harness.verification` merge shape) is unchanged. A finding
/// that actually gets re-graded this way logs a distinct
/// `"falsification-gate: regated (...)"` line rather than `"run"`, so a
/// caller can assert a regate genuinely happened (issue #119) — a finding
/// that was not already graded logs the ordinary `"run"` line regardless of
/// `regate`, since there was nothing to override.
///
/// Otherwise an explicit fixture entry's verdict is recorded as-is,
/// defaulting `attempted_at` to `now` when the fixture omits it (rather than
/// a fixed placeholder — a fixture author forgetting the field should record
/// the real grading time, not a value that reads as maximally stale to
/// freshness projections); a finding with no fixture entry **that was not
/// already graded** is recorded as a placeholder `inconclusive` (never a
/// false `survived`) that omits `attempted_at` entirely, so a later real
/// gate run can still overwrite it. A `regate` that produces this same
/// no-fixture-entry placeholder is the one exception: `attempted_at` is
/// still recorded, because the finding was already graded going into this
/// call — omitting it would defeat the one-round guard by making
/// `already_graded` read the finding as never-graded, letting a later
/// *non*-regate call re-grade it again. `now` is a caller-supplied parameter (mirroring
/// [`crate::resolve_membership`]'s `now`) rather than an internal clock
/// call, so tests stay deterministic; [`falsify`] is the real-clock
/// convenience wrapper most callers want.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `finding_path` cannot be read, and
/// [`MifRhError::Json`] if it is not valid JSON.
pub fn falsify_with_now(
    finding_path: &Path,
    fixture_path: Option<&Path>,
    now: chrono::DateTime<chrono::Utc>,
    regate: bool,
) -> Result<FalsifyResult, MifRhError> {
    let finding = read_json(finding_path)?;
    let id = finding
        .get("@id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let was_already_graded = already_graded(&finding);
    if was_already_graded && !regate {
        return Ok(FalsifyResult {
            finding,
            log_line: format!("falsification-gate: skipped (already falsified this session): {id}"),
        });
    }
    let regated = was_already_graded && regate;

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
    // A `regated` finding was already graded before this call, so it must
    // stay graded afterward — recording `attempted_at` even when this
    // regate produced a placeholder (no fixture entry) verdict. Without
    // this, `already_graded()` (which keys solely on `attempted_at`) would
    // read this finding as never-graded, defeating the one-round guard: a
    // later *non*-regate call would silently re-grade it again, exactly the
    // repeated-grading the one-round rule exists to prevent. A finding that
    // was never graded before this call keeps the placeholder-omits-
    // `attempted_at` behavior, so a later real fixture-backed run can still
    // overwrite it.
    if !placeholder || regated {
        verification.insert("attempted_at".to_string(), Value::String(attempted_at));
    }

    let updated = with_verification(finding, Value::Object(verification));
    let log_line = if regated {
        format!("falsification-gate: regated ({id} -> {verdict})")
    } else {
        format!("falsification-gate: run ({id} -> {verdict})")
    };
    Ok(FalsifyResult {
        finding: updated,
        log_line,
    })
}

#[cfg(test)]
mod tests {
    use super::falsify_with_now;
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

        let result = falsify_with_now(&finding, None, now(), false).unwrap();
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

        let result = falsify_with_now(&finding, Some(&fixture), now(), false).unwrap();
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

        let result = falsify_with_now(&finding, Some(&fixture), now(), false).unwrap();
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
        let result = falsify_with_now(&finding, None, now(), false).unwrap();
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

        let result = falsify_with_now(&finding, None, now(), false).unwrap();
        assert_eq!(result.finding["title"], "keep me");
        assert_eq!(result.finding["extensions"]["other"], "keep too");
        assert_eq!(
            result.finding["extensions"]["harness"]["verification"]["verdict"],
            "inconclusive"
        );
    }

    #[test]
    fn errors_on_a_missing_finding_file() {
        let error = falsify_with_now(
            std::path::Path::new("/no/such/file.json"),
            None,
            now(),
            false,
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::Io { .. }));
    }

    #[test]
    fn regate_bypasses_the_one_round_rule_and_logs_a_distinct_line() {
        let dir = tempfile::tempdir().unwrap();
        let finding = write(
            dir.path(),
            "f.json",
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"verification": {
                "verdict": "survived", "verdict_basis": "x", "attempted_at": "2026-01-01T00:00:00Z"
            }}}}"#,
        );
        let fixture = write(
            dir.path(),
            "evidence.json",
            r#"{"urn:mif:f1": {"verdict": "falsified", "basis": "new disconfirming evidence"}}"#,
        );

        let result = falsify_with_now(&finding, Some(&fixture), now(), true).unwrap();
        let verification = &result.finding["extensions"]["harness"]["verification"];
        assert_eq!(verification["verdict"], "falsified");
        assert_eq!(verification["verdict_basis"], "new disconfirming evidence");
        assert_eq!(verification["attempted_at"], "2026-06-01T00:00:00Z");
        assert_eq!(
            result.log_line,
            "falsification-gate: regated (urn:mif:f1 -> falsified)"
        );
    }

    #[test]
    fn regate_on_a_finding_never_graded_logs_the_ordinary_run_line() {
        // regate=true has nothing to override when the finding was never
        // graded in the first place -- it should behave identically to a
        // plain run, not fabricate a "regated" line.
        let dir = tempfile::tempdir().unwrap();
        let finding = write(dir.path(), "f.json", r#"{"@id": "urn:mif:f1"}"#);

        let result = falsify_with_now(&finding, None, now(), true).unwrap();
        assert_eq!(
            result.log_line,
            "falsification-gate: run (urn:mif:f1 -> inconclusive)"
        );
    }

    #[test]
    fn regate_with_no_fixture_entry_still_records_attempted_at_to_keep_the_one_round_guard() {
        // Regression for a review finding on #120: regate-ing an
        // already-graded finding against a fixture that has no entry for it
        // (or no fixture at all) produces the same placeholder `inconclusive`
        // verdict as a fresh, never-graded finding would. Unlike that fresh
        // case, this finding was ALREADY graded going into this call --
        // `attempted_at` must still be recorded, or `already_graded()` would
        // read it as never-graded and let a later non-regate call re-grade
        // it again, defeating the one-round rule the whole feature exists to
        // preserve exactly once past.
        let dir = tempfile::tempdir().unwrap();
        let finding = write(
            dir.path(),
            "f.json",
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"verification": {
                "verdict": "survived", "verdict_basis": "x", "attempted_at": "2026-01-01T00:00:00Z"
            }}}}"#,
        );

        let regated = falsify_with_now(&finding, None, now(), true).unwrap();
        let verification = &regated.finding["extensions"]["harness"]["verification"];
        assert_eq!(verification["verdict"], "inconclusive");
        assert_eq!(verification["attempted_at"], "2026-06-01T00:00:00Z");
        assert_eq!(
            regated.log_line,
            "falsification-gate: regated (urn:mif:f1 -> inconclusive)"
        );

        // The guard must hold afterward: write the regated result back out
        // and confirm a subsequent non-regate call skips it, not re-grades.
        fs::write(&finding, serde_json::to_string(&regated.finding).unwrap()).unwrap();
        let after = falsify_with_now(&finding, None, now(), false).unwrap();
        assert_eq!(after.finding, regated.finding);
        assert_eq!(
            after.log_line,
            "falsification-gate: skipped (already falsified this session): urn:mif:f1"
        );
    }
}
