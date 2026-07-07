//! Research-index incremental maintenance (rht Category B, Story #293).
//!
//! Ports rht's `scripts/build-index.sh`: a flat index of every MIF finding,
//! projecting the goal-version membership mirror (SPEC §11) from the
//! authoritative per-version `<findings-dir>/../goals/*.members.json` files.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde_json::{Value, json};

use crate::error::MifRhError;

struct Membership {
    goal_versions: BTreeSet<String>,
    stale_in: BTreeSet<String>,
}

/// Builds the flat research index from every `*.json` file directly under
/// `findings_dir`, folding in the goal-version membership mirror from
/// `<findings_dir>/../goals/*.members.json` (if any exist).
///
/// # Errors
///
/// Returns [`MifRhError::NoFindingsFound`] if `findings_dir` has no
/// `*.json` files, and [`MifRhError::Io`]/[`MifRhError::Json`] for
/// read/parse failures on either the findings or the members files.
pub fn build_index(findings_dir: &Path) -> Result<Value, MifRhError> {
    let findings = load_finding_files(findings_dir)?;
    let membership = fold_membership(findings_dir)?;

    let entries: Vec<Value> = findings
        .iter()
        .map(|finding| project_finding(finding, &membership))
        .collect();

    Ok(json!({
        "@type": "ResearchIndex",
        "count": entries.len(),
        "findings": entries,
    }))
}

fn load_finding_files(findings_dir: &Path) -> Result<Vec<Value>, MifRhError> {
    let mut paths: Vec<_> = std::fs::read_dir(findings_dir)
        .map_err(|source| MifRhError::Io {
            path: findings_dir.display().to_string(),
            source,
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(MifRhError::NoFindingsFound {
            path: findings_dir.display().to_string(),
        });
    }

    let mut findings = Vec::with_capacity(paths.len());
    for path in &paths {
        let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::FindingIo {
            path: path.display().to_string(),
            source,
        })?;
        let value: Value =
            serde_json::from_str(&contents).map_err(|source| MifRhError::FindingJson {
                path: path.display().to_string(),
                source,
            })?;
        findings.push(value);
    }
    Ok(findings)
}

/// Folds every `<findings_dir>/../goals/*.members.json` file into a
/// per-finding-id membership map. A members file with a non-string
/// `version` (legacy/partial) is skipped entirely, so a null version is
/// never projected as a fake goal-version id.
fn fold_membership(findings_dir: &Path) -> Result<HashMap<String, Membership>, MifRhError> {
    let goals_dir = findings_dir.join("../goals");
    let mut membership: HashMap<String, Membership> = HashMap::new();
    let Ok(entries) = std::fs::read_dir(&goals_dir) else {
        return Ok(membership);
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".members.json"))
        })
        .collect();
    paths.sort();

    for path in &paths {
        let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let doc: Value = serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
            path: path.display().to_string(),
            source,
        })?;
        let Some(version) = doc.get("version").and_then(Value::as_str) else {
            continue;
        };
        for member_id in doc
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = member_id.as_str() {
                membership
                    .entry(id.to_string())
                    .or_insert_with(|| Membership {
                        goal_versions: BTreeSet::new(),
                        stale_in: BTreeSet::new(),
                    })
                    .goal_versions
                    .insert(version.to_string());
            }
        }
        for stale_id in doc
            .get("stale")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = stale_id.as_str() {
                membership
                    .entry(id.to_string())
                    .or_insert_with(|| Membership {
                        goal_versions: BTreeSet::new(),
                        stale_in: BTreeSet::new(),
                    })
                    .stale_in
                    .insert(version.to_string());
            }
        }
    }
    Ok(membership)
}

fn project_finding(finding: &Value, membership: &HashMap<String, Membership>) -> Value {
    let id = finding
        .get("@id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let citations = finding
        .get("citations")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let entry = membership.get(id);
    json!({
        "id": id,
        "title": finding.get("title"),
        "namespace": finding.get("namespace"),
        "dimension": finding.pointer("/extensions/harness/dimension"),
        "tags": finding.get("tags").cloned().unwrap_or_else(|| json!([])),
        "verdict": finding.pointer("/extensions/harness/verification/verdict"),
        "citations": citations,
        "goal_versions": entry.map_or_else(Vec::new, |m| m.goal_versions.iter().cloned().collect::<Vec<_>>()),
        "stale_in": entry.map_or_else(Vec::new, |m| m.stale_in.iter().cloned().collect::<Vec<_>>()),
    })
}

#[cfg(test)]
mod tests {
    use super::build_index;
    use std::fs;

    #[test]
    fn projects_basic_finding_fields() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        fs::write(
            findings.join("f1.json"),
            r#"{"@id": "urn:mif:f1", "title": "T1", "namespace": "ns", "tags": ["a"],
                "citations": [{"url": "https://x"}, {"url": "https://y"}]}"#,
        )
        .unwrap();

        let index = build_index(&findings).unwrap();
        assert_eq!(index["count"], 1);
        let entry = &index["findings"][0];
        assert_eq!(entry["id"], "urn:mif:f1");
        assert_eq!(entry["title"], "T1");
        assert_eq!(entry["citations"], 2);
        assert_eq!(entry["goal_versions"], serde_json::json!([]));
    }

    #[test]
    fn folds_membership_from_goals_members_files() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        let goals = dir.path().join("goals");
        fs::create_dir_all(&findings).unwrap();
        fs::create_dir_all(&goals).unwrap();
        fs::write(findings.join("f1.json"), r#"{"@id": "urn:mif:f1"}"#).unwrap();
        fs::write(
            goals.join("goal-gv-1.members.json"),
            r#"{"version": "gv-1", "members": ["urn:mif:f1"], "stale": []}"#,
        )
        .unwrap();
        fs::write(
            goals.join("goal-gv-2.members.json"),
            r#"{"version": "gv-2", "members": ["urn:mif:f1"], "stale": ["urn:mif:f1"]}"#,
        )
        .unwrap();

        let index = build_index(&findings).unwrap();
        let entry = &index["findings"][0];
        assert_eq!(entry["goal_versions"], serde_json::json!(["gv-1", "gv-2"]));
        assert_eq!(entry["stale_in"], serde_json::json!(["gv-2"]));
    }

    #[test]
    fn skips_a_members_file_with_a_non_string_version() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        let goals = dir.path().join("goals");
        fs::create_dir_all(&findings).unwrap();
        fs::create_dir_all(&goals).unwrap();
        fs::write(findings.join("f1.json"), r#"{"@id": "urn:mif:f1"}"#).unwrap();
        fs::write(
            goals.join("goal-legacy.members.json"),
            r#"{"version": null, "members": ["urn:mif:f1"]}"#,
        )
        .unwrap();

        let index = build_index(&findings).unwrap();
        let entry = &index["findings"][0];
        assert_eq!(entry["goal_versions"], serde_json::json!([]));
    }

    #[test]
    fn rejects_an_empty_findings_directory() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();

        let error = build_index(&findings).unwrap_err();
        assert!(matches!(error, super::MifRhError::NoFindingsFound { .. }));
    }
}
