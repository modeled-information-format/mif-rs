//! Deterministic artifact synthesis from surviving findings (rht Category
//! B, Story #282).
//!
//! Ports rht's `scripts/synthesize-artifact.sh`: the report-synthesizer's
//! deterministic substrate (SPEC §6d). Consumes the surviving findings
//! (`verdict != "falsified"`) under a findings dir and produces one typed
//! Artifact (`schemas/artifact.schema.json`).

use std::path::Path;

use serde_json::{Value, json};

use crate::error::MifRhError;

fn is_falsified(finding: &Value) -> bool {
    finding
        .pointer("/extensions/harness/verification/verdict")
        .and_then(Value::as_str)
        .unwrap_or("survived")
        == "falsified"
}

fn citation_of(citation: &Value) -> Value {
    let mut out = json!({
        "title": citation.get("title"),
        "url": citation.get("url"),
        "citationType": citation.get("citationType").and_then(Value::as_str).unwrap_or("website"),
        "citationRole": citation.get("citationRole").and_then(Value::as_str).unwrap_or("supports"),
    });
    if let Some(note) = citation.get("note") {
        out["note"] = note.clone();
    }
    out
}

/// A resolved ontology mapping entry from `ontology-map.json`, keyed by
/// finding id.
struct OntologyMapEntry<'a> {
    entity_type: Option<&'a str>,
    resolved_ontology: Option<&'a str>,
    basis: Option<&'a Value>,
}

fn ontology_map_entry<'a>(
    ontology_map: &'a [Value],
    finding_id: &str,
) -> Option<OntologyMapEntry<'a>> {
    ontology_map
        .iter()
        .find(|entry| entry.get("finding_id").and_then(Value::as_str) == Some(finding_id))
        .map(|entry| OntologyMapEntry {
            entity_type: entry.get("entity_type").and_then(Value::as_str),
            resolved_ontology: entry.get("resolved_ontology").and_then(Value::as_str),
            basis: entry.get("basis"),
        })
}

fn section_of(finding: &Value, ontology_map: &[Value]) -> Value {
    let finding_id = finding
        .get("@id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body = finding
        .get("content")
        .or_else(|| finding.get("summary"))
        .or_else(|| finding.get("title"))
        .cloned()
        .unwrap_or(Value::Null);
    let sources: Vec<Value> = finding
        .get("citations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(citation_of)
        .collect();
    let entities: Vec<Value> = finding
        .get("entities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|entity| {
            json!({
                "id": entity.pointer("/entity/@id"),
                "name": entity.get("name"),
                "entityType": entity.get("entityType").and_then(Value::as_str).unwrap_or("entity"),
            })
        })
        .collect();
    let relationships: Vec<Value> = finding
        .get("relationships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|relationship| {
            let rel_type = relationship
                .get("type")
                .and_then(Value::as_str)
                .or_else(|| relationship.get("relationshipType").and_then(Value::as_str))
                .unwrap_or("relates-to");
            let target = relationship.get("target").and_then(|t| {
                if t.is_object() {
                    t.get("@id").cloned()
                } else {
                    Some(t.clone())
                }
            });
            let mut out = json!({ "type": rel_type, "target": target });
            if let Some(strength) = relationship.get("strength") {
                out["strength"] = strength.clone();
            }
            out
        })
        .collect();

    let mut section = json!({
        "heading": finding.get("title"),
        "body": body,
        "supports": [finding_id],
        "sources": sources,
        "entities": entities,
        "relationships": relationships,
        "dimension": finding.pointer("/extensions/harness/dimension").and_then(Value::as_str).unwrap_or("general"),
        "verdict": finding
            .pointer("/extensions/harness/verification/verdict")
            .and_then(Value::as_str)
            .unwrap_or("inconclusive"),
    });
    if let Some(entry) = ontology_map_entry(ontology_map, finding_id) {
        if let Some(entity_type) = entry.entity_type {
            section["entityType"] = json!(entity_type);
        }
        if let Some(resolved_ontology) = entry.resolved_ontology {
            section["ontology"] = json!(resolved_ontology);
        }
        if let Some(basis) = entry.basis {
            section["basis"] = basis.clone();
        }
    }
    section
}

/// Deduplicates citations by URL, keeping (per group) the one with the
/// longest `note`.
///
/// On a tie, the LAST citation (in the original list's order) wins —
/// matching jq's `max_by`, whose stable sort picks the final element
/// among equal keys, not the first.
fn dedupe_sources(sources: Vec<Value>) -> Vec<Value> {
    use std::collections::BTreeMap;
    let mut by_url: BTreeMap<String, Value> = BTreeMap::new();
    let note_len = |source: &Value| -> usize {
        source
            .get("note")
            .and_then(Value::as_str)
            .map_or(0, str::len)
    };
    for source in sources {
        let url = source
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match by_url.get(&url) {
            Some(existing) if note_len(existing) > note_len(&source) => {},
            _ => {
                by_url.insert(url, source);
            },
        }
    }
    by_url.into_values().collect()
}

/// Synthesizes an Artifact from every surviving (non-falsified) finding
/// directly under `findings_dir`.
///
/// # Errors
///
/// Returns [`MifRhError::NoFindingsFound`] if `findings_dir` has no
/// `*.json` files, [`MifRhError::NoSurvivingFindings`] if every finding is
/// falsified, and [`MifRhError::ArtifactNotPublishable`] if the result has
/// no sections, finding refs, or sources.
pub fn synthesize_artifact(findings_dir: &Path, genre: &str) -> Result<Value, MifRhError> {
    let findings = load_findings(findings_dir)?;
    let ontology_map = load_ontology_map(findings_dir);

    let surviving: Vec<&Value> = findings.iter().filter(|f| !is_falsified(f)).collect();
    if surviving.is_empty() {
        return Err(MifRhError::NoSurvivingFindings {
            path: findings_dir.display().to_string(),
        });
    }

    let namespace = surviving[0]
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or("harness/report");
    let title = format!("Findings: {namespace}");
    let finding_refs: Vec<&str> = surviving
        .iter()
        .filter_map(|f| f.get("@id").and_then(Value::as_str))
        .collect();
    let sections: Vec<Value> = surviving
        .iter()
        .map(|f| section_of(f, &ontology_map))
        .collect();
    let all_citations: Vec<Value> = surviving
        .iter()
        .flat_map(|f| {
            f.get("citations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .map(citation_of)
        .collect();
    let sources = dedupe_sources(all_citations);

    let artifact = json!({
        "@type": "Artifact",
        "title": title,
        "genre": genre,
        "audience": "general",
        "newsworthiness": "medium",
        "namespace": namespace,
        "mif": { "conformanceLevel": 3 },
        "finding_refs": finding_refs,
        "sections": sections,
        "sources": sources,
    });

    if sections.is_empty()
        || finding_refs.is_empty()
        || artifact["sources"].as_array().is_none_or(Vec::is_empty)
    {
        return Err(MifRhError::ArtifactNotPublishable {
            path: findings_dir.display().to_string(),
        });
    }
    Ok(artifact)
}

fn load_findings(findings_dir: &Path) -> Result<Vec<Value>, MifRhError> {
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

/// Loads `<findings_dir>/../ontology-map.json` if present, non-empty, and
/// a valid JSON array; falls back to an empty map on anything else
/// (missing, corrupt, or a placeholder), matching the original's
/// byte-identical no-map path.
fn load_ontology_map(findings_dir: &Path) -> Vec<Value> {
    let path = findings_dir.join("../ontology-map.json");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    if contents.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Value>(&contents)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::synthesize_artifact;
    use std::fs;

    fn write_finding(dir: &std::path::Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn synthesizes_one_section_per_surviving_finding() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        write_finding(
            &findings,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "Finding One", "namespace": "harness/x",
                "content": "body text",
                "citations": [{"title": "Src", "url": "https://a.example"}]}"#,
        );

        let artifact = synthesize_artifact(&findings, "general").unwrap();
        assert_eq!(artifact["sections"].as_array().unwrap().len(), 1);
        assert_eq!(artifact["sections"][0]["heading"], "Finding One");
        assert_eq!(artifact["title"], "Findings: harness/x");
    }

    #[test]
    fn excludes_falsified_findings() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        write_finding(
            &findings,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "Good", "namespace": "harness/x",
                "content": "b", "citations": [{"title": "S", "url": "https://a.example"}]}"#,
        );
        write_finding(
            &findings,
            "f2.json",
            r#"{"@id": "urn:mif:f2", "title": "Bad", "namespace": "harness/x", "content": "b",
                "extensions": {"harness": {"verification": {"verdict": "falsified"}}}}"#,
        );

        let artifact = synthesize_artifact(&findings, "general").unwrap();
        assert_eq!(artifact["sections"].as_array().unwrap().len(), 1);
        assert_eq!(artifact["sections"][0]["heading"], "Good");
    }

    #[test]
    fn rejects_a_findings_set_that_is_entirely_falsified() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        write_finding(
            &findings,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "Bad", "namespace": "harness/x",
                "extensions": {"harness": {"verification": {"verdict": "falsified"}}}}"#,
        );

        let error = synthesize_artifact(&findings, "general").unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::NoSurvivingFindings { .. }
        ));
    }

    #[test]
    fn rejects_an_artifact_with_no_sources_as_unpublishable() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        write_finding(
            &findings,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "No citations", "namespace": "harness/x", "content": "b"}"#,
        );

        let error = synthesize_artifact(&findings, "general").unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::ArtifactNotPublishable { .. }
        ));
    }

    #[test]
    fn dedupes_sources_by_url_keeping_the_longest_note() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        write_finding(
            &findings,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "F1", "namespace": "harness/x", "content": "b",
                "citations": [
                    {"title": "S", "url": "https://a.example", "note": "short"},
                    {"title": "S", "url": "https://a.example", "note": "a much longer note here"}
                ]}"#,
        );

        let artifact = synthesize_artifact(&findings, "general").unwrap();
        let sources = artifact["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0]["note"], "a much longer note here");
    }

    /// Regression test: jq's `max_by` is a stable sort that keeps the
    /// LAST element among tied keys, not the first. An earlier Rust
    /// implementation got this backwards (first-wins), which was only
    /// caught by a real-corpus parity diff against the original script —
    /// with no note on either citation (both length 0, a genuine tie),
    /// the fixture below reproduces the exact failure mode.
    #[test]
    fn dedupe_sources_breaks_a_note_length_tie_in_favor_of_the_last_citation() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        write_finding(
            &findings,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "F1", "namespace": "harness/x", "content": "b",
                "citations": [
                    {"title": "First (no note)", "url": "https://tie.example"},
                    {"title": "Second (no note)", "url": "https://tie.example"},
                    {"title": "Third (no note)", "url": "https://tie.example"}
                ]}"#,
        );

        let artifact = synthesize_artifact(&findings, "general").unwrap();
        let sources = artifact["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0]["title"], "Third (no note)");
    }

    #[test]
    fn folds_in_an_ontology_map_entry_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let findings = dir.path().join("findings");
        fs::create_dir_all(&findings).unwrap();
        write_finding(
            &findings,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "F1", "namespace": "harness/x", "content": "b",
                "citations": [{"title": "S", "url": "https://a.example"}]}"#,
        );
        fs::write(
            dir.path().join("ontology-map.json"),
            r#"[{"finding_id": "urn:mif:f1", "entity_type": "tool", "resolved_ontology": "software", "basis": "exact"}]"#,
        )
        .unwrap();

        let artifact = synthesize_artifact(&findings, "general").unwrap();
        assert_eq!(artifact["sections"][0]["entityType"], "tool");
        assert_eq!(artifact["sections"][0]["ontology"], "software");
        assert_eq!(artifact["sections"][0]["basis"], "exact");
    }
}
