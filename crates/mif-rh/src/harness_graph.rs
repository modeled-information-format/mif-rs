//! MIF-native knowledge-graph construction (rht Category B, Story #293).
//!
//! Ports rht's `scripts/build-graph.sh`: derives a first-class knowledge
//! graph from MIF `EntityReference`s and typed relationships, never from
//! tag co-occurrence. Every node and edge traces to a `urn:mif:` id.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use crate::error::MifRhError;

/// One finding JSON file's fields relevant to graph construction.
struct FindingRef<'a> {
    id: &'a str,
    title: Option<&'a str>,
    dimension: Option<&'a str>,
    entities: &'a [Value],
    relationships: &'a [Value],
}

fn as_finding(value: &Value) -> Option<FindingRef<'_>> {
    let id = value.get("@id").and_then(Value::as_str)?;
    Some(FindingRef {
        id,
        title: value.get("title").and_then(Value::as_str),
        dimension: value
            .pointer("/extensions/harness/dimension")
            .and_then(Value::as_str),
        entities: value
            .get("entities")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice),
        relationships: value
            .get("relationships")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice),
    })
}

fn target_id(target: &Value) -> Option<&str> {
    if target.is_object() {
        target.get("@id").and_then(Value::as_str)
    } else {
        target.as_str()
    }
}

/// Builds the MIF-native knowledge graph from every `*.json` finding file
/// directly under `findings_dir` (non-recursive, matching the original
/// script's `-maxdepth 1`).
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `findings_dir` cannot be read, and
/// [`MifRhError::FindingJson`] if a finding file is not valid JSON or has
/// no string `@id`.
pub fn build_graph(findings_dir: &Path) -> Result<Value, MifRhError> {
    let findings = load_findings(findings_dir)?;
    let refs: Vec<FindingRef<'_>> = findings.iter().filter_map(as_finding).collect();

    let (mut node_order, node_by_id) = collect_nodes(&refs);
    let (edges, relationship_targets) = collect_edges(&refs);

    // `unique_by(.id)` sorts by id (stable-deduplicating within-group,
    // keeping the first pre-sort occurrence) — sort $known nodes to match.
    node_order.sort();
    let mut nodes: Vec<Value> = node_order.iter().map(|id| node_by_id[id].clone()).collect();
    for target in &relationship_targets {
        if !node_by_id.contains_key(target) {
            nodes.push(json!({
                "id": target,
                "kind": "concept",
                "label": target,
                "external": true,
            }));
        }
    }

    Ok(json!({
        "@type": "KnowledgeGraph",
        "generator": "build-graph.sh (MIF-native; SPEC §6c)",
        "nodes": nodes,
        "edges": edges,
    }))
}

/// Reads every `*.json` file directly under `findings_dir` (non-recursive),
/// sorted by path.
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
    if findings.is_empty() {
        return Err(MifRhError::NoFindingsFound {
            path: findings_dir.display().to_string(),
        });
    }
    Ok(findings)
}

/// Collects one concept node per finding and one entity node per distinct
/// referenced MIF entity (concepts win on an id collision, matching the
/// `$concepts + $entities` concatenation order jq's `unique_by` dedupes
/// against).
fn collect_nodes(
    refs: &[FindingRef<'_>],
) -> (Vec<String>, std::collections::HashMap<String, Value>) {
    let mut node_order: Vec<String> = Vec::new();
    let mut node_by_id: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let mut insert_node = |id: String, node: Value| {
        if let std::collections::hash_map::Entry::Vacant(entry) = node_by_id.entry(id.clone()) {
            node_order.push(id);
            entry.insert(node);
        }
    };

    for finding in refs {
        insert_node(
            finding.id.to_string(),
            json!({
                "id": finding.id,
                "kind": "concept",
                "label": finding.title.unwrap_or(finding.id),
                "dimension": finding.dimension,
            }),
        );
    }
    for finding in refs {
        for entity in finding.entities {
            let Some(entity_id) = entity.pointer("/entity/@id").and_then(Value::as_str) else {
                continue;
            };
            insert_node(
                entity_id.to_string(),
                json!({
                    "id": entity_id,
                    "kind": "entity",
                    "label": entity.get("name").and_then(Value::as_str).unwrap_or(entity_id),
                    "entityType": entity.get("entityType").and_then(Value::as_str),
                }),
            );
        }
    }
    (node_order, node_by_id)
}

/// Two full passes (relationship edges across ALL findings, then mention
/// edges across ALL findings), matching jq's `$reledges + $mentions` — NOT
/// interleaved per finding. Also returns every relationship target id, so
/// the caller can materialize external stub nodes for any target outside
/// the known corpus.
fn collect_edges(refs: &[FindingRef<'_>]) -> (Vec<Value>, BTreeSet<String>) {
    let mut relationship_targets: BTreeSet<String> = BTreeSet::new();
    let mut edges = Vec::new();
    for finding in refs {
        for relationship in finding.relationships {
            let Some(target) = relationship.get("target").and_then(target_id) else {
                continue;
            };
            relationship_targets.insert(target.to_string());
            let edge_type = relationship
                .get("type")
                .and_then(Value::as_str)
                .or_else(|| relationship.get("relationshipType").and_then(Value::as_str));
            edges.push(json!({
                "source": finding.id,
                "target": target,
                "type": edge_type,
                "strength": relationship.get("strength"),
                "via": "relationship",
            }));
        }
    }
    for finding in refs {
        for entity in finding.entities {
            let Some(entity_id) = entity.pointer("/entity/@id").and_then(Value::as_str) else {
                continue;
            };
            edges.push(json!({
                "source": finding.id,
                "target": entity_id,
                "type": "mentions",
                "strength": Value::Null,
                "via": "entity",
            }));
        }
    }
    (edges, relationship_targets)
}

#[cfg(test)]
mod tests {
    use super::build_graph;
    use std::fs;

    fn write_finding(dir: &std::path::Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn builds_concept_and_entity_nodes_with_mention_edges() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "Finding One",
                "entities": [{"entity": {"@id": "urn:mif:entity:tool:widget"}, "name": "Widget", "entityType": "tool"}]}"#,
        );

        let graph = build_graph(dir.path()).unwrap();
        let nodes = graph["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        let edges = graph["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["type"], "mentions");
        assert_eq!(edges[0]["source"], "urn:mif:f1");
        assert_eq!(edges[0]["target"], "urn:mif:entity:tool:widget");
    }

    #[test]
    fn deduplicates_entities_referenced_by_multiple_findings() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "f1.json",
            r#"{"@id": "urn:mif:f1", "entities": [{"entity": {"@id": "urn:mif:entity:tool:widget"}, "name": "Widget"}]}"#,
        );
        write_finding(
            dir.path(),
            "f2.json",
            r#"{"@id": "urn:mif:f2", "entities": [{"entity": {"@id": "urn:mif:entity:tool:widget"}, "name": "Widget"}]}"#,
        );

        let graph = build_graph(dir.path()).unwrap();
        let entity_count = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["kind"] == "entity")
            .count();
        assert_eq!(entity_count, 1);
    }

    #[test]
    fn materializes_an_external_stub_for_a_cross_corpus_relationship_target() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "f1.json",
            r#"{"@id": "urn:mif:f1", "relationships": [{"target": "urn:mif:outside", "type": "supports"}]}"#,
        );

        let graph = build_graph(dir.path()).unwrap();
        let external: Vec<_> = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["external"] == true)
            .collect();
        assert_eq!(external.len(), 1);
        assert_eq!(external[0]["id"], "urn:mif:outside");
    }

    #[test]
    fn resolves_a_structured_relationship_target_object() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "f1.json",
            r#"{"@id": "urn:mif:f1", "relationships": [{"target": {"@id": "urn:mif:f2"}, "type": "contradicts"}]}"#,
        );
        write_finding(dir.path(), "f2.json", r#"{"@id": "urn:mif:f2"}"#);

        let graph = build_graph(dir.path()).unwrap();
        let edges = graph["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["target"], "urn:mif:f2");
        // f2 is already a known concept node, so it must NOT also appear as
        // an external stub.
        let external_count = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["external"] == true)
            .count();
        assert_eq!(external_count, 0);
    }
}
