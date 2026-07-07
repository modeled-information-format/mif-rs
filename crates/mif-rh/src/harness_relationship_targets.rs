//! Corpus-wide relationship-target integrity gate (rht Category B, Story #287).
//!
//! Ports rht's `scripts/check-relationship-targets.sh`: proves every
//! `relationships[].target` across the ACTIVE corpus (every
//! `<topic>/findings/*.json`, corpus-wide — `quarantine/`/`archive/`
//! siblings are deliberately excluded) resolves to a real, active finding
//! `@id`. `@id` is a globally unique URN, so the id universe spans every
//! topic, not just the target's own.
//!
//! A finding file that fails to parse hard-fails the whole gate (never
//! silently skipped) — an unparseable file could hide a real dangling
//! target elsewhere in the corpus if it were dropped from either universe.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::MifRhError;
use crate::harness_project::read_json;

/// One orphaned `relationships[].target`: the finding file it was declared
/// in, and the target value that resolves to no active finding `@id`.
#[derive(Debug, Clone)]
pub struct Orphan {
    /// The finding file path (as discovered, not canonicalized).
    pub source_file: String,
    /// The dangling target value.
    pub target: String,
}

/// One [`check_relationship_targets`] run's result.
#[derive(Debug, Clone)]
pub struct RelationshipTargetsReport {
    /// Every orphaned target, sorted by `(source_file, target)` (the
    /// original script's `find`-derived order is filesystem-dependent and
    /// not itself part of the contract).
    pub orphans: Vec<Orphan>,
    /// Total non-empty `relationships[].target` values checked (not
    /// deduplicated — matches the original script's `checked` count).
    pub checked: usize,
    /// The number of unique active finding `@id`s in the corpus.
    pub active_findings: usize,
}

impl RelationshipTargetsReport {
    /// Whether every relationship target resolved (no orphans).
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.orphans.is_empty()
    }
}

/// Every `<reports_dir>/<topic>/findings/*.json` file, corpus-wide
/// (mirrors the original script's `find -mindepth 2 -maxdepth 2 -type d
/// -name findings` then one level into each). Hidden files are excluded;
/// a `.tmp` partial write's name never ends in `.json`, so it is excluded
/// by the extension filter alone, same as the original.
fn active_finding_paths(reports_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let Ok(topics) = std::fs::read_dir(reports_dir) else {
        return paths;
    };
    for topic in topics.flatten() {
        let findings_dir = topic.path().join("findings");
        let Ok(entries) = std::fs::read_dir(&findings_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_json_file = path.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with('.'));
            if is_json_file {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

fn relationship_targets(finding: &Value) -> Vec<String> {
    finding
        .get("relationships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|relationship| relationship.get("target").and_then(Value::as_str))
        .filter(|target| !target.is_empty())
        .map(str::to_string)
        .collect()
}

/// Checks every `relationships[].target` in the active corpus under
/// `reports_dir` against the corpus-wide active `@id` universe.
///
/// # Errors
///
/// Returns [`MifRhError::RelationshipTargetFindingUnparseable`] if any
/// active finding file cannot be parsed — a malformed finding anywhere in
/// the corpus hard-fails the whole gate rather than silently narrowing the
/// id/target universes and missing a real orphan past it.
pub fn check_relationship_targets(
    reports_dir: &Path,
) -> Result<RelationshipTargetsReport, MifRhError> {
    let paths = active_finding_paths(reports_dir);
    if paths.is_empty() {
        return Ok(RelationshipTargetsReport {
            orphans: Vec::new(),
            checked: 0,
            active_findings: 0,
        });
    }

    let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_targets: Vec<(String, String)> = Vec::new();
    for path in &paths {
        let finding =
            read_json(path).map_err(|_| MifRhError::RelationshipTargetFindingUnparseable {
                path: path.display().to_string(),
            })?;
        if let Some(id) = finding.get("@id").and_then(Value::as_str)
            && !id.is_empty()
        {
            ids.insert(id.to_string());
        }
        let file = path.display().to_string();
        for target in relationship_targets(&finding) {
            all_targets.push((file.clone(), target));
        }
    }

    let mut orphans: Vec<Orphan> = all_targets
        .iter()
        .filter(|(_, target)| !ids.contains(target))
        .map(|(source_file, target)| Orphan {
            source_file: source_file.clone(),
            target: target.clone(),
        })
        .collect();
    orphans.sort_by(|a, b| (&a.source_file, &a.target).cmp(&(&b.source_file, &b.target)));

    Ok(RelationshipTargetsReport {
        orphans,
        checked: all_targets.len(),
        active_findings: ids.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::check_relationship_targets;
    use std::fs;

    fn write(dir: &std::path::Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn passes_when_every_target_resolves() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "t1/findings/a.json",
            r#"{"@id":"urn:mif:t1:a","relationships":[]}"#,
        );
        write(
            dir.path(),
            "t1/findings/b.json",
            r#"{"@id":"urn:mif:t1:b","relationships":[{"type":"relates-to","target":"urn:mif:t1:a"}]}"#,
        );

        let report = check_relationship_targets(dir.path()).unwrap();
        assert!(report.ok(), "{:?}", report.orphans);
        assert_eq!(report.checked, 1);
        assert_eq!(report.active_findings, 2);
    }

    #[test]
    fn flags_a_dangling_target_and_a_quarantine_only_target() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "t1/findings/a.json",
            r#"{"@id":"urn:mif:t1:a","relationships":[]}"#,
        );
        write(
            dir.path(),
            "t1/findings/b.json",
            r#"{"@id":"urn:mif:t1:b","relationships":[
                {"type":"relates-to","target":"urn:mif:t1:does-not-exist"},
                {"type":"relates-to","target":"urn:mif:t1:quarantined"}
            ]}"#,
        );
        write(
            dir.path(),
            "t1/quarantine/quarantined.json",
            r#"{"@id":"urn:mif:t1:quarantined","relationships":[]}"#,
        );

        let report = check_relationship_targets(dir.path()).unwrap();
        assert!(!report.ok());
        let targets: Vec<&str> = report.orphans.iter().map(|o| o.target.as_str()).collect();
        assert!(targets.contains(&"urn:mif:t1:does-not-exist"));
        assert!(targets.contains(&"urn:mif:t1:quarantined"));
    }

    #[test]
    fn resolves_targets_across_topics_since_id_is_a_global_urn() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "t1/findings/a.json",
            r#"{"@id":"urn:mif:t1:a","relationships":[]}"#,
        );
        write(
            dir.path(),
            "t2/findings/b.json",
            r#"{"@id":"urn:mif:t2:b","relationships":[{"type":"relates-to","target":"urn:mif:t1:a"}]}"#,
        );

        let report = check_relationship_targets(dir.path()).unwrap();
        assert!(report.ok(), "{:?}", report.orphans);
    }

    #[test]
    fn a_malformed_finding_hard_fails_instead_of_silently_narrowing_the_universe() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "t1/findings/bad.json", "{invalid json");
        write(
            dir.path(),
            "t1/findings/z.json",
            r#"{"@id":"urn:mif:t1:z","relationships":[{"type":"relates-to","target":"urn:mif:t1:does-not-exist"}]}"#,
        );

        let error = check_relationship_targets(dir.path()).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::RelationshipTargetFindingUnparseable { .. }
        ));
    }

    #[test]
    fn no_active_findings_passes_with_zero_checked() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("t1")).unwrap();

        let report = check_relationship_targets(dir.path()).unwrap();
        assert!(report.ok());
        assert_eq!(report.checked, 0);
        assert_eq!(report.active_findings, 0);
    }
}
