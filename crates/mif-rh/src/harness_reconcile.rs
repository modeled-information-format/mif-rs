//! Durable session checkpoint + remaining-work plan (rht Category B,
//! Story #282).
//!
//! Ports rht's `scripts/reconcile-session.sh` (SPEC §6b): derives
//! `reports/<topic>/state.json` purely from disk. A finding is DONE iff
//! it validates against `schemas/findings.schema.json` (which requires
//! `extensions.harness.verification` — verdict + `verdict_basis`), so a
//! valid finding has already been through the falsification gate.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::MifRhError;
use crate::harness_project::{read_json, validate_against_schema};

/// One finding's reconciled state: id, dimension, schema validity,
/// verification timestamp/verdict.
struct FindingRecord {
    id: String,
    dimension: String,
    valid: bool,
    attempted_at: Option<String>,
    verdict: Option<String>,
}

impl FindingRecord {
    fn is_done(&self) -> bool {
        self.valid && self.verdict.as_deref() != Some("falsified")
    }

    fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "dimension": self.dimension,
            "valid": self.valid,
            "attempted_at": self.attempted_at,
            "verdict": self.verdict,
        })
    }
}

/// The result of a [`reconcile_session`] call.
#[derive(Debug)]
pub struct ReconcileReport {
    /// The full `state.json` contents (also written to disk).
    pub state: Value,
    /// The remaining-work plan, one line per item, sorted. Empty means
    /// nothing remains.
    pub plan: Vec<String>,
}

/// Every real finding file under `reports_dir`: canonical
/// `findings/*.json` plus a defensive flat `finding-*.json`, sorted and
/// deduplicated by path. Hidden (`.`-prefixed) and `*.tmp` files are
/// in-flight partial writes and are excluded.
pub(crate) fn list_finding_paths(reports_dir: &Path) -> Vec<PathBuf> {
    let mut paths: HashSet<PathBuf> = HashSet::new();
    let findings_dir = reports_dir.join("findings");
    if let Ok(entries) = std::fs::read_dir(&findings_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if is_real_finding_file(&path, "") {
                paths.insert(path);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(reports_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if is_real_finding_file(&path, "finding-") {
                paths.insert(path);
            }
        }
    }
    let mut paths: Vec<PathBuf> = paths.into_iter().collect();
    paths.sort();
    paths
}

fn is_real_finding_file(path: &Path, required_prefix: &str) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    path.is_file()
        && path.extension().is_some_and(|ext| ext == "json")
        && !name.starts_with('.')
        && path.extension().is_none_or(|ext| ext != "tmp")
        && name.starts_with(required_prefix)
}

fn count_partial_writes(reports_dir: &Path) -> usize {
    let mut count = 0;
    for dir in [reports_dir.join("findings"), reports_dir.to_path_buf()] {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            count += entries
                .flatten()
                .filter(|entry| {
                    let path = entry.path();
                    path.is_file() && path.extension().is_some_and(|ext| ext == "tmp")
                })
                .count();
        }
    }
    count
}

/// Collapses duplicate `@id`s (the same finding can appear in both
/// `findings/` and the flat legacy path): one record per id, preferring a
/// DONE copy, else a valid copy, else the first seen — matching jq's
/// `group_by(.id) | map((select(isdone)|first) // (select(.valid)|first)
/// // .[0])`.
fn collapse_duplicates(records: Vec<FindingRecord>) -> Vec<FindingRecord> {
    let mut by_id: BTreeMap<String, Vec<FindingRecord>> = BTreeMap::new();
    for record in records {
        by_id.entry(record.id.clone()).or_default().push(record);
    }
    by_id
        .into_values()
        .map(|mut group| {
            let done_index = group.iter().position(FindingRecord::is_done);
            let valid_index = group.iter().position(|r| r.valid);
            let index = done_index.or(valid_index).unwrap_or(0);
            group.swap_remove(index)
        })
        .collect()
}

/// Reconciles the session at `reports_dir` (`reports/<topic>`), writing
/// `state.json` and returning it plus the remaining-work plan.
///
/// # Errors
///
/// Returns [`MifRhError::ReconcileEnvironmentBroken`] if the known-good
/// sample finding at `sample_finding_path` fails to validate (the schema
/// toolchain itself is broken — this must never be read as "every finding
/// is invalid, re-run everything"), and [`MifRhError::Io`]/
/// [`MifRhError::Json`] for read/write failures.
pub fn reconcile_session(
    reports_dir: &Path,
    schema_path: &Path,
    ref_paths: &[PathBuf],
    sample_finding_path: &Path,
) -> Result<ReconcileReport, MifRhError> {
    let sample = read_json(sample_finding_path)?;
    if validate_against_schema(&sample, sample_finding_path, schema_path, ref_paths).is_err() {
        return Err(MifRhError::ReconcileEnvironmentBroken {
            sample_path: sample_finding_path.display().to_string(),
        });
    }

    let topic = reports_dir
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let paths = list_finding_paths(reports_dir);
    let partial_count = count_partial_writes(reports_dir);

    let mut records = Vec::with_capacity(paths.len());
    for path in &paths {
        let Ok(finding) = read_json(path) else {
            continue;
        };
        let id = finding
            .get("@id")
            .and_then(Value::as_str)
            .or_else(|| finding.get("id").and_then(Value::as_str))
            .map_or_else(
                || {
                    path.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                },
                str::to_string,
            );
        let dimension = finding
            .pointer("/extensions/harness/dimension")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let attempted_at = finding
            .pointer("/extensions/harness/verification/attempted_at")
            .and_then(Value::as_str)
            .map(str::to_string);
        let verdict = finding
            .pointer("/extensions/harness/verification/verdict")
            .and_then(Value::as_str)
            .map(str::to_string);
        let valid = validate_against_schema(&finding, path, schema_path, ref_paths).is_ok();
        records.push(FindingRecord {
            id,
            dimension,
            valid,
            attempted_at,
            verdict,
        });
    }

    let mut findings = collapse_duplicates(records);
    findings.sort_by(|a, b| a.id.cmp(&b.id));

    let mut dimension_totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for finding in &findings {
        let entry = dimension_totals
            .entry(finding.dimension.clone())
            .or_insert((0, 0));
        entry.0 += 1;
        if finding.is_done() {
            entry.1 += 1;
        }
    }
    let dimensions: Value = dimension_totals
        .iter()
        .map(|(dim, (total, done))| (dim.clone(), json!({ "total": total, "done": done })))
        .collect::<serde_json::Map<_, _>>()
        .into();

    let findings_present = findings.iter().any(FindingRecord::is_done);
    let no_invalid_findings = findings.iter().all(|f| f.valid);
    let no_partial_writes = partial_count == 0;

    let mut checks = vec![
        json!({ "check": "findings_present", "passed": findings_present }),
        json!({ "check": "no_invalid_findings", "passed": no_invalid_findings }),
        json!({ "check": "no_partial_writes", "passed": no_partial_writes }),
    ];
    checks.sort_by(|a, b| a["check"].as_str().cmp(&b["check"].as_str()));

    let mut state = json!({
        "topic": topic,
        "findings": findings.iter().map(FindingRecord::to_value).collect::<Vec<_>>(),
        "dimensions": dimensions,
        "checks": checks,
    });

    if let Some(concordance) = project_concordance_status(reports_dir, &paths) {
        state["concordance"] = concordance;
    }
    // Matches jq's `-S`: recursively sort every object's keys for
    // byte-deterministic output.
    let state = sort_object_keys(&state);

    write_state(reports_dir, &state)?;
    let plan = compute_plan(&state);

    Ok(ReconcileReport { state, plan })
}

/// Recursively sorts every JSON object's keys (matching jq's `-S` flag),
/// leaving arrays' element order and scalar values untouched.
#[must_use]
pub fn sort_object_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), sort_object_keys(v)))
                .collect();
            sorted.into_iter().collect::<serde_json::Map<_, _>>().into()
        },
        Value::Array(items) => Value::Array(items.iter().map(sort_object_keys).collect()),
        other => other.clone(),
    }
}

/// Projects the cross-topic concordance's status into the checkpoint,
/// only when `../concordance.json` exists (a deliberate,
/// existence-guarded exception to "purely from `reports/<topic>`").
fn project_concordance_status(reports_dir: &Path, finding_paths: &[PathBuf]) -> Option<Value> {
    let concordance_path = reports_dir.join("../concordance.json");
    let concordance = read_json(&concordance_path).ok()?;
    let node_count = concordance["nodes"].as_array().map_or(0, Vec::len);
    let edge_count = concordance["edges"].as_array().map_or(0, Vec::len);

    let status_path = reports_dir.join("../concordance-status.json");
    let valid = read_json(&status_path)
        .ok()
        .and_then(|s| s.get("valid").cloned())
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // mapok requires the file to exist, parse as JSON, AND be an array —
    // `read_json` enforces the first two; `.as_array()` narrows the third.
    let ontology_map_path = reports_dir.join("ontology-map.json");
    let ontology_map: Option<Vec<Value>> = read_json(&ontology_map_path)
        .ok()
        .and_then(|v| v.as_array().cloned());

    let untyped_shippable = count_untyped_shippable(finding_paths, ontology_map.as_deref());

    Some(json!({
        "built": true,
        "valid": valid,
        "nodes": node_count,
        "edges": edge_count,
        "untyped_shippable": untyped_shippable,
    }))
}

/// Counts unique shippable (survived|weakened) findings whose ontology-map
/// record is missing/invalid/untyped/unresolved/discovery-only. An
/// unparseable finding or a finding with no `@id` is blocked by the ship
/// gate too, so it counts here as untyped (keyed by path so it's never
/// dropped). A missing/unparseable map means EVERY shippable finding is
/// untyped (fail-closed).
///
/// A well-formed finding with no verdict at all (a raw/in-progress finding,
/// not yet through the falsification gate) is neither shippable nor
/// unreadable — `check-shippable-typing.sh`'s `jq -er '... // ""'` reads a
/// missing verdict as `""`, which is not an error, and `""` never matches
/// `survived|weakened`, so it is skipped, not blocked. Only a finding whose
/// JSON itself fails to parse counts as unreadable.
fn count_untyped_shippable(finding_paths: &[PathBuf], ontology_map: Option<&[Value]>) -> usize {
    let mut keys: HashSet<String> = HashSet::new();
    for path in finding_paths {
        let Ok(finding) = read_json(path) else {
            keys.insert(format!("unreadable:{}", path.display()));
            continue;
        };
        let verdict = finding
            .pointer("/extensions/harness/verification/verdict")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if verdict != "survived" && verdict != "weakened" {
            continue;
        }
        let fid = finding.get("@id").and_then(Value::as_str);
        keys.insert(fid.map_or_else(|| format!("noid:{}", path.display()), str::to_string));
    }

    keys.into_iter()
        .filter(|key| {
            if key.starts_with("noid:") || key.starts_with("unreadable:") {
                return true;
            }
            let Some(ontology_map) = ontology_map else {
                return true;
            };
            let record = ontology_map.iter().find(|entry| {
                entry.get("finding_id").and_then(Value::as_str) == Some(key.as_str())
            });
            record.is_none_or(|record| {
                let valid = record
                    .get("valid")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let basis = record
                    .get("basis")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                !valid || matches!(basis, "untyped" | "unresolved" | "discovery")
            })
        })
        .count()
}

fn write_state(reports_dir: &Path, state: &Value) -> Result<(), MifRhError> {
    let staging = reports_dir.join(".state.json.staging");
    let text = serde_json::to_string_pretty(state).map_err(|source| MifRhError::JsonSerialize {
        path: staging.display().to_string(),
        source,
    })?;
    std::fs::write(&staging, format!("{text}\n")).map_err(|source| MifRhError::Io {
        path: staging.display().to_string(),
        source,
    })?;
    let final_path = reports_dir.join("state.json");
    std::fs::rename(&staging, &final_path).map_err(|source| MifRhError::Io {
        path: final_path.display().to_string(),
        source,
    })
}

fn compute_plan(state: &Value) -> Vec<String> {
    let mut plan: Vec<String> = Vec::new();
    if let Some(dimensions) = state.get("dimensions").and_then(Value::as_object) {
        for (dim, counts) in dimensions {
            let total = counts.get("total").and_then(Value::as_u64).unwrap_or(0);
            let done = counts.get("done").and_then(Value::as_u64).unwrap_or(0);
            if done < total {
                plan.push(format!(
                    "dimension {dim}: {} finding(s) need work",
                    total - done
                ));
            }
        }
    }
    if let Some(checks) = state.get("checks").and_then(Value::as_array) {
        for check in checks {
            if check.get("passed").and_then(Value::as_bool) == Some(false) {
                let name = check
                    .get("check")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                plan.push(format!("check {name}: FAIL"));
            }
        }
    }
    plan.sort();
    plan
}

#[cfg(test)]
mod tests {
    use super::reconcile_session;
    use std::fs;

    const FINDINGS_SCHEMA: &str = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["@id", "extensions"],
        "properties": {
            "@id": {"type": "string"},
            "extensions": {
                "type": "object",
                "required": ["harness"],
                "properties": {
                    "harness": {
                        "type": "object",
                        "required": ["dimension", "verification"],
                        "properties": {
                            "dimension": {"type": "string"},
                            "verification": {
                                "type": "object",
                                "required": ["verdict", "verdict_basis"],
                                "properties": {
                                    "verdict": {"type": "string"},
                                    "verdict_basis": {"type": "string"}
                                }
                            }
                        }
                    }
                }
            }
        }
    }"#;

    const VALID_FINDING: &str = r#"{"@id": "urn:mif:f1",
        "extensions": {"harness": {"dimension": "landscape",
            "verification": {"verdict": "survived", "verdict_basis": "b", "attempted_at": "2026-01-01"}}}}"#;

    fn setup(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let schema_path = dir.join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let sample_path = dir.join("sample.json");
        fs::write(&sample_path, VALID_FINDING).unwrap();
        (schema_path, sample_path)
    }

    #[test]
    fn a_valid_survived_finding_counts_as_done() {
        let dir = tempfile::tempdir().unwrap();
        let (schema_path, sample_path) = setup(dir.path());
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        fs::write(reports_dir.join("findings/f1.json"), VALID_FINDING).unwrap();

        let report = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap();
        assert_eq!(report.state["dimensions"]["landscape"]["total"], 1);
        assert_eq!(report.state["dimensions"]["landscape"]["done"], 1);
        assert!(report.plan.is_empty());
    }

    #[test]
    fn a_falsified_finding_is_valid_but_not_done() {
        let dir = tempfile::tempdir().unwrap();
        let (schema_path, sample_path) = setup(dir.path());
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        fs::write(
            reports_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"dimension": "landscape",
                "verification": {"verdict": "falsified", "verdict_basis": "b"}}}}"#,
        )
        .unwrap();

        let report = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap();
        assert_eq!(report.state["dimensions"]["landscape"]["done"], 0);
        assert!(
            report
                .plan
                .iter()
                .any(|l| l.contains("dimension landscape"))
        );
    }

    #[test]
    fn an_invalid_finding_fails_the_no_invalid_findings_check() {
        let dir = tempfile::tempdir().unwrap();
        let (schema_path, sample_path) = setup(dir.path());
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        fs::write(
            reports_dir.join("findings/f1.json"),
            r#"{"@id": "urn:mif:f1"}"#,
        )
        .unwrap();

        let report = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap();
        let checks = report.state["checks"].as_array().unwrap();
        let no_invalid = checks
            .iter()
            .find(|c| c["check"] == "no_invalid_findings")
            .unwrap();
        assert_eq!(no_invalid["passed"], false);
        assert!(
            report
                .plan
                .iter()
                .any(|l| l.contains("no_invalid_findings"))
        );
    }

    #[test]
    fn a_partial_tmp_write_fails_the_no_partial_writes_check() {
        let dir = tempfile::tempdir().unwrap();
        let (schema_path, sample_path) = setup(dir.path());
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        fs::write(reports_dir.join("findings/f1.json"), VALID_FINDING).unwrap();
        fs::write(reports_dir.join("findings/f2.json.tmp"), "partial").unwrap();

        let report = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap();
        let checks = report.state["checks"].as_array().unwrap();
        let no_partial = checks
            .iter()
            .find(|c| c["check"] == "no_partial_writes")
            .unwrap();
        assert_eq!(no_partial["passed"], false);
    }

    #[test]
    fn a_duplicate_id_prefers_the_done_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (schema_path, sample_path) = setup(dir.path());
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        fs::write(reports_dir.join("findings/f1.json"), VALID_FINDING).unwrap();
        // A flat legacy duplicate of the same @id, invalid this time.
        fs::write(
            reports_dir.join("finding-f1-legacy.json"),
            r#"{"@id": "urn:mif:f1"}"#,
        )
        .unwrap();

        let report = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap();
        let findings = report.state["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["valid"], true);
    }

    #[test]
    fn rejects_a_broken_environment_where_the_sample_itself_fails_validation() {
        let dir = tempfile::tempdir().unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let sample_path = dir.path().join("sample.json");
        fs::write(&sample_path, r#"{"not": "a valid finding"}"#).unwrap();
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(&reports_dir).unwrap();

        let error = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::ReconcileEnvironmentBroken { .. }
        ));
    }

    #[test]
    fn projects_concordance_status_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let (schema_path, sample_path) = setup(dir.path());
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        fs::write(reports_dir.join("findings/f1.json"), VALID_FINDING).unwrap();
        fs::write(
            dir.path().join("reports/concordance.json"),
            r#"{"nodes": [1, 2, 3], "edges": [1]}"#,
        )
        .unwrap();

        let report = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap();
        assert_eq!(report.state["concordance"]["built"], true);
        assert_eq!(report.state["concordance"]["nodes"], 3);
        assert_eq!(report.state["concordance"]["edges"], 1);
    }

    #[test]
    fn untyped_shippable_does_not_count_a_raw_finding_with_no_verdict() {
        // check-shippable-typing.sh reads a missing verdict as "" (jq's `// ""`
        // on a nonexistent path is not an error) and "" never matches
        // survived|weakened, so a raw/in-progress finding is skipped, not
        // blocked. untyped_shippable must mirror that, not over-count it as
        // unreadable.
        let dir = tempfile::tempdir().unwrap();
        let (schema_path, sample_path) = setup(dir.path());
        let reports_dir = dir.path().join("reports/topic");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        fs::write(
            reports_dir.join("findings/raw.json"),
            r#"{"@id": "urn:mif:raw"}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("reports/concordance.json"),
            r#"{"nodes": [], "edges": []}"#,
        )
        .unwrap();

        let report = reconcile_session(&reports_dir, &schema_path, &[], &sample_path).unwrap();
        assert_eq!(report.state["concordance"]["untyped_shippable"], 0);
    }
}
