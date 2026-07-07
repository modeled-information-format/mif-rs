//! Deterministic goal-version scope resolution (rht Category B, Story
//! #293).
//!
//! Ports rht's `scripts/resolve-membership.sh` (SPEC §11): classifies a
//! topic's existing findings against a goal version's contract and emits
//! the authoritative per-version members file. This is the deterministic
//! floor; ambiguous in/out-of-scope judgement is layered on top elsewhere.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value, json};

use crate::error::MifRhError;

const DEFAULT_FRESHNESS_DAYS: f64 = 180.0;

/// One finding's scope/staleness classification.
struct Classified {
    id: String,
    dimension: Option<String>,
    in_scope: bool,
    stale: bool,
}

fn compute_ttl_days(citations: &[Value], by_citation_type: &Value, default_days: f64) -> f64 {
    citations
        .iter()
        .filter_map(|citation| citation.get("citationType").and_then(Value::as_str))
        .map(|citation_type| {
            by_citation_type
                .get(citation_type)
                .and_then(Value::as_f64)
                .unwrap_or(default_days)
        })
        .fold(None, |min: Option<f64>, days| {
            Some(min.map_or(days, |current| current.min(days)))
        })
        .unwrap_or(default_days)
}

/// Whether `attempted_at` (an RFC 3339-ish timestamp, possibly with an
/// offset or fractional seconds) is more than `ttl_days` in the past,
/// relative to `now`. A missing or unparseable `attempted_at` is always
/// stale (freshness-unknown), matching the original's `$t == null then
/// true` branch.
// Freshness windows are measured in whole days, so truncating the
// sub-second remainder of `ttl_days * 86400` is exactly the precision the
// domain calls for, not a bug.
#[allow(clippy::cast_possible_truncation)]
fn is_stale(attempted_at: Option<&str>, ttl_days: f64, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(attempted_at) = attempted_at else {
        return true;
    };
    let Some(date_part) = attempted_at.get(0..10) else {
        return true;
    };
    let Ok(date) = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d") else {
        return true;
    };
    let Some(midnight) = date.and_hms_opt(0, 0, 0) else {
        return true;
    };
    let midnight_utc = midnight.and_utc();
    let threshold = midnight_utc + chrono::Duration::seconds((ttl_days * 86400.0) as i64);
    now > threshold
}

fn classify_finding(
    finding: &Value,
    dims: &[String],
    fresh: &Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Classified {
    let id = finding
        .get("@id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let dimension = finding
        .pointer("/extensions/harness/dimension")
        .and_then(Value::as_str)
        .map(str::to_string);
    let verdict = finding
        .pointer("/extensions/harness/verification/verdict")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let in_scope = verdict != "falsified"
        && dimension
            .as_deref()
            .is_some_and(|d| dims.iter().any(|dim| dim == d));

    let stale = if in_scope {
        let default_days = fresh
            .get("default_days")
            .and_then(Value::as_f64)
            .unwrap_or(DEFAULT_FRESHNESS_DAYS);
        let by_citation_type = fresh
            .get("by_citation_type")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let citations = finding
            .get("citations")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let ttl_days = compute_ttl_days(citations, &by_citation_type, default_days);
        let attempted_at = finding
            .pointer("/extensions/harness/verification/attempted_at")
            .and_then(Value::as_str);
        is_stale(attempted_at, ttl_days, now)
    } else {
        false
    };

    Classified {
        id,
        dimension,
        in_scope,
        stale,
    }
}

/// The result of a [`resolve_membership`] call.
pub struct MembershipReport {
    /// The members file's full JSON contents (also written to disk).
    pub members_file: Value,
    /// The path it was written to.
    pub out_path: std::path::PathBuf,
}

/// Resolves goal-version scope for `topic` and writes the authoritative
/// members file `<topic_dir>/goals/goal-<version>.members.json`.
///
/// # Errors
///
/// Returns [`MifRhError::Io`]/[`MifRhError::Json`] for read/write/parse
/// failures on the goal, config, findings, or existing members file.
pub fn resolve_membership(
    topic_dir: &Path,
    config_path: &Path,
    version: &str,
    generated: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<MembershipReport, MifRhError> {
    let goal_path = topic_dir.join("goal.json");
    let goal = read_json(&goal_path)?;
    let dims: Vec<String> = goal
        .get("dimensions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| d.as_str())
        .map(str::to_string)
        .collect();

    let fresh = read_json(config_path)
        .ok()
        .and_then(|config| config.get("freshness").cloned())
        .unwrap_or_else(|| json!({}));

    let findings_dir = topic_dir.join("findings");
    let findings = load_findings_if_present(&findings_dir)?;
    let classified: Vec<Classified> = findings
        .iter()
        .map(|f| classify_finding(f, &dims, &fresh, now))
        .collect();

    let out_dir = topic_dir.join("goals");
    std::fs::create_dir_all(&out_dir).map_err(|source| MifRhError::Io {
        path: out_dir.display().to_string(),
        source,
    })?;
    let out_path = out_dir.join(format!("goal-{version}.members.json"));

    let excluded: HashSet<String> = read_json(&out_path)
        .ok()
        .and_then(|doc| doc.get("excluded").cloned())
        .and_then(|v| v.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    let kept: Vec<&Classified> = classified
        .iter()
        .filter(|c| c.in_scope)
        .filter(|c| !excluded.contains(&c.id))
        .collect();
    let members: Vec<String> = kept.iter().map(|c| c.id.clone()).collect();
    let stale: Vec<String> = kept
        .iter()
        .filter(|c| c.stale)
        .map(|c| c.id.clone())
        .collect();
    let covered_dims: HashSet<&str> = kept.iter().filter_map(|c| c.dimension.as_deref()).collect();
    let gap_dimensions: Vec<String> = dims
        .iter()
        .filter(|d| !covered_dims.contains(d.as_str()))
        .cloned()
        .collect();

    let members_file = json!({
        "version": version,
        "generated": generated,
        "members": members,
        "stale": stale,
        "excluded": excluded.into_iter().collect::<Vec<_>>(),
        "gap_dimensions": gap_dimensions,
    });

    write_json_pretty(&out_path, &members_file)?;

    Ok(MembershipReport {
        members_file,
        out_path,
    })
}

fn load_findings_if_present(findings_dir: &Path) -> Result<Vec<Value>, MifRhError> {
    let Ok(entries) = std::fs::read_dir(findings_dir) else {
        return Ok(Vec::new());
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();

    let mut findings = Vec::with_capacity(paths.len());
    for path in &paths {
        findings.push(read_json(path)?);
    }
    Ok(findings)
}

fn read_json(path: &Path) -> Result<Value, MifRhError> {
    let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
        path: path.display().to_string(),
        source,
    })
}

fn write_json_pretty(path: &Path, value: &Value) -> Result<(), MifRhError> {
    let text = serde_json::to_string_pretty(value).map_err(|source| MifRhError::JsonSerialize {
        path: path.display().to_string(),
        source,
    })?;
    std::fs::write(path, format!("{text}\n")).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::resolve_membership;
    use std::fs;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .to_utc()
    }

    fn setup(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let topic_dir = dir.join("reports/topic");
        let findings_dir = topic_dir.join("findings");
        fs::create_dir_all(&findings_dir).unwrap();
        fs::write(
            topic_dir.join("goal.json"),
            r#"{"dimensions": ["landscape", "trajectory"]}"#,
        )
        .unwrap();
        let config_path = dir.join("harness.config.json");
        fs::write(&config_path, r#"{"freshness": {"default_days": 30}}"#).unwrap();
        (topic_dir, config_path)
    }

    #[test]
    fn keeps_an_in_scope_survived_finding_as_a_member() {
        let dir = tempfile::tempdir().unwrap();
        let (topic_dir, config_path) = setup(dir.path());
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "survived", "attempted_at": "2026-05-30"}}}}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(
            report.members_file["members"],
            serde_json::json!(["urn:mif:f1"])
        );
        assert_eq!(report.members_file["stale"], serde_json::json!([]));
    }

    #[test]
    fn excludes_a_falsified_finding_regardless_of_dimension() {
        let dir = tempfile::tempdir().unwrap();
        let (topic_dir, config_path) = setup(dir.path());
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "falsified"}}}}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(report.members_file["members"], serde_json::json!([]));
    }

    #[test]
    fn a_finding_with_no_attempted_at_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let (topic_dir, config_path) = setup(dir.path());
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "survived"}}}}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(
            report.members_file["stale"],
            serde_json::json!(["urn:mif:f1"])
        );
    }

    #[test]
    fn a_finding_past_its_ttl_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let (topic_dir, config_path) = setup(dir.path());
        // default_days = 30, attempted 60 days before `now` (2026-06-01).
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "survived", "attempted_at": "2026-04-01"}}}}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(
            report.members_file["stale"],
            serde_json::json!(["urn:mif:f1"])
        );
    }

    #[test]
    fn a_recently_verified_finding_within_ttl_is_not_stale() {
        let dir = tempfile::tempdir().unwrap();
        let (topic_dir, config_path) = setup(dir.path());
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "survived", "attempted_at": "2026-05-25"}}}}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(report.members_file["stale"], serde_json::json!([]));
    }

    #[test]
    fn by_citation_type_ttl_overrides_the_default_via_the_minimum() {
        let dir = tempfile::tempdir().unwrap();
        let topic_dir = dir.path().join("reports/topic");
        let findings_dir = topic_dir.join("findings");
        fs::create_dir_all(&findings_dir).unwrap();
        fs::write(
            topic_dir.join("goal.json"),
            r#"{"dimensions": ["landscape"]}"#,
        )
        .unwrap();
        let config_path = dir.path().join("harness.config.json");
        // default 365 days, but "flaky" citations get only a 5-day TTL —
        // the finding's TTL is the MIN over its citation types.
        fs::write(
            &config_path,
            r#"{"freshness": {"default_days": 365, "by_citation_type": {"flaky": 5}}}"#,
        )
        .unwrap();
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "citations": [{"citationType": "flaky"}],
                "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "survived", "attempted_at": "2026-05-20"}}}}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(
            report.members_file["stale"],
            serde_json::json!(["urn:mif:f1"])
        );
    }

    #[test]
    fn a_gap_dimension_with_no_in_scope_finding_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (topic_dir, config_path) = setup(dir.path());
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "survived", "attempted_at": "2026-05-30"}}}}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(
            report.members_file["gap_dimensions"],
            serde_json::json!(["trajectory"])
        );
    }

    #[test]
    fn re_resolving_honors_a_prior_manual_exclusion() {
        let dir = tempfile::tempdir().unwrap();
        let (topic_dir, config_path) = setup(dir.path());
        fs::write(
            topic_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "survived", "attempted_at": "2026-05-30"}}}}"#,
        )
        .unwrap();
        let goals_dir = topic_dir.join("goals");
        fs::create_dir_all(&goals_dir).unwrap();
        fs::write(
            goals_dir.join("goal-gv-1.members.json"),
            r#"{"version": "gv-1", "members": [], "stale": [], "excluded": ["urn:mif:f1"], "gap_dimensions": []}"#,
        )
        .unwrap();

        let report = resolve_membership(
            &topic_dir,
            &config_path,
            "gv-1",
            "2026-06-01T00:00:00Z",
            now(),
        )
        .unwrap();
        assert_eq!(report.members_file["members"], serde_json::json!([]));
        assert_eq!(
            report.members_file["excluded"],
            serde_json::json!(["urn:mif:f1"])
        );
    }
}
