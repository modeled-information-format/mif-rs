//! Fail-closed pre-synthesis typing gate (rht Category B, Story #287, ADR-0011).
//!
//! Ports rht's `scripts/check-shippable-typing.sh`: a finding that SHIPS
//! (verdict `survived`/`weakened`) must resolve to a valid ontology type.
//! Untyped/unresolved/invalid/missing-from-map/discovery-only shippable
//! findings, and any finding whose JSON cannot even be parsed, block
//! synthesis. This covers a gap `validate-concordance.sh` cannot see: an
//! untyped finding's concept node gets `entityType: null`, and
//! `validate-concordance.sh` filters on `entityType != null`, so an untyped
//! shippable finding would otherwise pass the spine validator vacuously.

use std::path::Path;

use serde_json::Value;

use crate::error::MifRhError;
use crate::harness_project::read_json;
use crate::harness_reconcile::list_finding_paths;

/// One [`check_shippable_typing`] run's result: every blocker line, in
/// discovery order (sorted by path, matching the original script's `sort -u`).
#[derive(Debug, Clone)]
pub struct ShippableTypingReport {
    /// One `"  {id-or-path} ({reason})"` line per blocking finding.
    pub blockers: Vec<String>,
}

impl ShippableTypingReport {
    /// Whether every shippable finding carries a valid ontology type (no
    /// blockers).
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.blockers.is_empty()
    }
}

fn load_ontology_map(reports_dir: &Path, topic: &str) -> Result<Vec<Value>, MifRhError> {
    let map_path = reports_dir.join("ontology-map.json");
    if !map_path.is_file() {
        return Err(MifRhError::OntologyMapUnusable {
            path: map_path.display().to_string(),
            topic: topic.to_string(),
            reason: "is missing".to_string(),
        });
    }
    read_json(&map_path)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .ok_or_else(|| MifRhError::OntologyMapUnusable {
            path: map_path.display().to_string(),
            topic: topic.to_string(),
            reason: "is unparseable or not a record array".to_string(),
        })
}

/// `None` when the record resolves the finding as typed and shippable;
/// `Some(reason)` when it should block, matching
/// `reconcile-session.sh`/`count_untyped_shippable`'s exact predicate.
fn blocking_reason(map: &[Value], raw_id: &str) -> Option<String> {
    let record = map
        .iter()
        .find(|entry| entry.get("finding_id").and_then(Value::as_str) == Some(raw_id));
    record.map_or_else(
        || Some("missing".to_string()),
        |record| {
            let valid = record
                .get("valid")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let basis = record.get("basis").and_then(Value::as_str).unwrap_or("");
            if !valid {
                Some("invalid".to_string())
            } else if matches!(basis, "untyped" | "unresolved" | "discovery") {
                Some(basis.to_string())
            } else {
                None
            }
        },
    )
}

fn check_finding(path: &Path, map: &[Value]) -> Option<String> {
    let Ok(finding) = read_json(path) else {
        return Some(format!("  {} (unreadable JSON)", path.display()));
    };
    let verdict = finding
        .pointer("/extensions/harness/verification/verdict")
        .and_then(Value::as_str)
        .unwrap_or("");
    if verdict != "survived" && verdict != "weakened" {
        return None;
    }
    // `.["@id"] // empty` in jq yields nothing (captured as "" by the bash
    // command substitution) when absent — `${id:-$f}` then falls back to
    // the file path for display. An empty id is also used as-is for the
    // map lookup (matching the original, however unlikely a real match).
    let raw_id = finding.get("@id").and_then(Value::as_str).unwrap_or("");
    let label = if raw_id.is_empty() {
        path.display().to_string()
    } else {
        raw_id.to_string()
    };
    blocking_reason(map, raw_id).map(|reason| format!("  {label} ({reason})"))
}

/// Checks every shippable finding under `reports_dir` (e.g. `reports/<topic>`)
/// for a valid ontology type.
///
/// # Errors
///
/// Returns [`MifRhError::OntologyMapUnusable`] if `reports_dir`'s
/// `ontology-map.json` is missing or not a JSON array of records — without
/// it, typing cannot be proven for any finding, so the gate fails closed
/// rather than passing vacuously.
pub fn check_shippable_typing(reports_dir: &Path) -> Result<ShippableTypingReport, MifRhError> {
    let topic = reports_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let map = load_ontology_map(reports_dir, topic)?;

    let blockers = list_finding_paths(reports_dir)
        .iter()
        .filter_map(|path| check_finding(path, &map))
        .collect();

    Ok(ShippableTypingReport { blockers })
}

#[cfg(test)]
mod tests {
    use super::check_shippable_typing;
    use std::fs;

    fn setup(dir: &std::path::Path) -> std::path::PathBuf {
        let reports_dir = dir.join("reports/edu");
        fs::create_dir_all(reports_dir.join("findings")).unwrap();
        reports_dir
    }

    #[test]
    fn passes_a_fully_typed_shippable_finding() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());
        fs::write(
            reports_dir.join("findings/f1.json"),
            r#"{"@id":"urn:mif:f1","extensions":{"harness":{"verification":{"verdict":"survived"}}}}"#,
        )
        .unwrap();
        fs::write(
            reports_dir.join("ontology-map.json"),
            r#"[{"finding_id":"urn:mif:f1","valid":true,"basis":"declared"}]"#,
        )
        .unwrap();

        let report = check_shippable_typing(&reports_dir).unwrap();
        assert!(report.ok(), "{:?}", report.blockers);
    }

    #[test]
    fn blocks_an_untyped_shippable_finding() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());
        fs::write(
            reports_dir.join("findings/f1.json"),
            r#"{"@id":"urn:mif:f1","extensions":{"harness":{"verification":{"verdict":"survived"}}}}"#,
        )
        .unwrap();
        fs::write(
            reports_dir.join("ontology-map.json"),
            r#"[{"finding_id":"urn:mif:f1","valid":true,"basis":"untyped"}]"#,
        )
        .unwrap();

        let report = check_shippable_typing(&reports_dir).unwrap();
        assert_eq!(report.blockers, vec!["  urn:mif:f1 (untyped)"]);
    }

    #[test]
    fn blocks_a_discovery_only_shippable_finding_not_just_untyped_or_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());
        fs::write(
            reports_dir.join("findings/f1.json"),
            r#"{"@id":"urn:mif:f1","extensions":{"harness":{"verification":{"verdict":"survived"}}}}"#,
        )
        .unwrap();
        fs::write(
            reports_dir.join("ontology-map.json"),
            r#"[{"finding_id":"urn:mif:f1","valid":true,"basis":"discovery"}]"#,
        )
        .unwrap();

        let report = check_shippable_typing(&reports_dir).unwrap();
        assert_eq!(report.blockers, vec!["  urn:mif:f1 (discovery)"]);
    }

    #[test]
    fn a_falsified_untyped_finding_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());
        fs::write(
            reports_dir.join("findings/f1.json"),
            r#"{"@id":"urn:mif:f1","extensions":{"harness":{"verification":{"verdict":"falsified"}}}}"#,
        )
        .unwrap();
        fs::write(
            reports_dir.join("ontology-map.json"),
            r#"[{"finding_id":"urn:mif:f1","valid":true,"basis":"untyped"}]"#,
        )
        .unwrap();

        let report = check_shippable_typing(&reports_dir).unwrap();
        assert!(report.ok());
    }

    #[test]
    fn an_unparseable_finding_blocks_closed() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());
        fs::write(
            reports_dir.join("findings/corrupt.json"),
            "{ not valid json ",
        )
        .unwrap();
        fs::write(reports_dir.join("ontology-map.json"), "[]").unwrap();

        let report = check_shippable_typing(&reports_dir).unwrap();
        assert_eq!(report.blockers.len(), 1);
        assert!(report.blockers[0].contains("unreadable JSON"));
    }

    #[test]
    fn a_missing_ontology_map_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());

        let error = check_shippable_typing(&reports_dir).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::OntologyMapUnusable { reason, .. } if reason == "is missing"
        ));
    }

    #[test]
    fn a_wrong_shape_ontology_map_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());
        fs::write(
            reports_dir.join("ontology-map.json"),
            r#"{"not":"an array"}"#,
        )
        .unwrap();

        let error = check_shippable_typing(&reports_dir).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::OntologyMapUnusable { reason, .. }
                if reason == "is unparseable or not a record array"
        ));
    }

    #[test]
    fn a_no_id_shippable_finding_blocks_and_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = setup(dir.path());
        fs::write(
            reports_dir.join("findings/noid.json"),
            r#"{"extensions":{"harness":{"verification":{"verdict":"survived"}}}}"#,
        )
        .unwrap();
        fs::write(reports_dir.join("ontology-map.json"), "[]").unwrap();

        let report = check_shippable_typing(&reports_dir).unwrap();
        assert_eq!(report.blockers.len(), 1);
        assert!(report.blockers[0].contains("noid.json"));
        assert!(report.blockers[0].contains("(missing)"));
    }

    #[test]
    fn a_flat_finding_layout_is_gated_too() {
        let dir = tempfile::tempdir().unwrap();
        let reports_dir = dir.path().join("reports/edu");
        fs::create_dir_all(&reports_dir).unwrap();
        fs::write(
            reports_dir.join("finding-flat.json"),
            r#"{"@id":"urn:mif:flat","extensions":{"harness":{"verification":{"verdict":"survived"}}}}"#,
        )
        .unwrap();
        fs::write(
            reports_dir.join("ontology-map.json"),
            r#"[{"finding_id":"urn:mif:flat","valid":true,"basis":"untyped"}]"#,
        )
        .unwrap();

        let report = check_shippable_typing(&reports_dir).unwrap();
        assert_eq!(report.blockers, vec!["  urn:mif:flat (untyped)"]);
    }
}
