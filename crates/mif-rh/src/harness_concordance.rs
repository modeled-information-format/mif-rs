//! The ontological spine: one unified, cross-topic concordance (rht
//! Category B, Story #282).
//!
//! Ports rht's `scripts/build-concordance.sh` (SPEC §8d): merges every
//! topic's findings into a single MIF-native graph typed by the ontology.
//! Concept nodes (one per finding) carry their resolved ontology
//! `entity_type` (from `reports/<topic>/ontology-map.json`) and
//! falsification verdict; entity nodes merge across topics by `urn:mif:`
//! `@id`. All findings become nodes — falsified ones are flagged, not
//! excluded.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde_json::{Value, json};

use crate::error::MifRhError;

fn target_id(target: &Value) -> Option<&str> {
    if target.is_object() {
        target.get("@id").and_then(Value::as_str)
    } else {
        target.as_str()
    }
}

fn load_findings(findings_dir: &Path) -> Result<Vec<Value>, MifRhError> {
    let mut paths: Vec<_> = std::fs::read_dir(findings_dir)
        .map_err(|source| MifRhError::Io {
            path: findings_dir.display().to_string(),
            source,
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json") && !path.ends_with(".tmp"))
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
    Ok(findings)
}

fn load_ontology_map(reports_dir: &Path, topic: &str) -> Vec<Value> {
    let path = reports_dir.join(topic).join("ontology-map.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

fn ontology_map_lookup<'a>(ontology_map: &'a [Value], finding_id: &str) -> Option<&'a Value> {
    ontology_map
        .iter()
        .find(|entry| entry.get("finding_id").and_then(Value::as_str) == Some(finding_id))
}

/// Sorted topic directory names directly under `reports_dir` that have a
/// `findings/` subdirectory (matching bash's sorted glob
/// `"$RD"/*/findings`).
fn list_topics(reports_dir: &Path) -> Result<Vec<String>, MifRhError> {
    let mut topics: Vec<String> = std::fs::read_dir(reports_dir)
        .map_err(|source| MifRhError::Io {
            path: reports_dir.display().to_string(),
            source,
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("findings").is_dir())
        .filter_map(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    topics.sort();
    Ok(topics)
}

struct TopicGraph {
    concepts: Vec<Value>,
    entities: Vec<Value>,
    edges: Vec<Value>,
}

fn build_topic_graph(reports_dir: &Path, topic: &str) -> Result<TopicGraph, MifRhError> {
    let findings = load_findings(&reports_dir.join(topic).join("findings"))?;
    let ontology_map = load_ontology_map(reports_dir, topic);

    let mut concepts = Vec::with_capacity(findings.len());
    let mut entities = Vec::new();
    let mut reledges = Vec::new();
    let mut mentions = Vec::new();

    for finding in &findings {
        let finding_id = finding
            .get("@id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let verdict = finding.pointer("/extensions/harness/verification/verdict");
        let om = ontology_map_lookup(&ontology_map, finding_id);
        let entity_type = om
            .and_then(|entry| entry.get("entity_type"))
            .filter(|v| !v.is_null())
            .or_else(|| finding.pointer("/entity/entity_type"))
            .cloned()
            .unwrap_or(Value::Null);
        let ontology = om
            .and_then(|entry| entry.get("resolved_ontology"))
            .cloned()
            .unwrap_or(Value::Null);
        let flagged = verdict.and_then(Value::as_str) == Some("falsified");
        concepts.push(json!({
            "id": finding_id,
            "kind": "concept",
            "label": finding.get("title").cloned().unwrap_or_else(|| json!(finding_id)),
            "topics": [topic],
            "entityType": entity_type,
            "ontology": ontology,
            "verdict": verdict.cloned().unwrap_or(Value::Null),
            "flagged": flagged,
        }));

        for entity in finding
            .get("entities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(entity_id) = entity.pointer("/entity/@id").and_then(Value::as_str) else {
                continue;
            };
            entities.push(json!({
                "id": entity_id,
                "kind": "entity",
                "label": entity.get("name").cloned().unwrap_or_else(|| json!(entity_id)),
                "entityType": entity.get("entityType").cloned().unwrap_or(Value::Null),
                "topics": [topic],
                "flagged": false,
            }));
            mentions.push(json!({
                "source": finding_id,
                "target": entity_id,
                "type": "mentions",
                "strength": Value::Null,
                "via": "entity",
            }));
        }

        for relationship in finding
            .get("relationships")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(target) = relationship.get("target").and_then(target_id) else {
                continue;
            };
            let edge_type = relationship
                .get("type")
                .and_then(Value::as_str)
                .or_else(|| relationship.get("relationshipType").and_then(Value::as_str));
            reledges.push(json!({
                "source": finding_id,
                "target": target,
                "type": edge_type,
                "strength": relationship.get("strength").cloned().unwrap_or(Value::Null),
                "via": "relationship",
            }));
        }
    }

    let mut edges = reledges;
    edges.extend(mentions);
    Ok(TopicGraph {
        concepts,
        entities,
        edges,
    })
}

/// Merges a flat list of `{id, topics: [...], ...}` node objects by `id`:
/// the first occurrence's fields win (matching jq's `group_by(.id) |
/// map(.[0] + {topics: union})`, whose sorted-stable `group_by` keeps the
/// first pre-sort occurrence per group), with `topics` replaced by the
/// sorted union across every occurrence of that id.
fn merge_nodes_by_id(nodes: Vec<Value>) -> Vec<Value> {
    let mut first_seen: HashMap<String, Value> = HashMap::new();
    let mut topics_by_id: HashMap<String, BTreeSet<String>> = HashMap::new();
    for node in nodes {
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let node_topics: Vec<String> = node
            .get("topics")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect();
        topics_by_id
            .entry(id.clone())
            .or_default()
            .extend(node_topics);
        first_seen.entry(id).or_insert(node);
    }
    first_seen
        .into_iter()
        .map(|(id, mut node)| {
            let topics: Vec<Value> = topics_by_id
                .remove(&id)
                .unwrap_or_default()
                .into_iter()
                .map(Value::String)
                .collect();
            node["topics"] = Value::Array(topics);
            node
        })
        .collect()
}

/// Deduplicates edges by full-object equality (an edge with a different
/// `strength` is NOT a duplicate, matching jq's plain `unique`, which
/// compares whole objects).
fn dedupe_edges(edges: Vec<Value>) -> Vec<Value> {
    let mut seen: HashSet<String> = HashSet::new();
    edges
        .into_iter()
        .filter(|edge| seen.insert(serde_json::to_string(edge).unwrap_or_default()))
        .collect()
}

fn edge_sort_key(edge: &Value) -> (String, String, String, String) {
    let field = |name: &str| {
        edge.get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    (
        field("source"),
        field("target"),
        field("type"),
        field("via"),
    )
}

/// Builds the cross-topic concordance from every `<reports_dir>/<topic>/findings/`
/// directory.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `reports_dir` cannot be read, and
/// [`MifRhError::FindingJson`] if a finding file is not valid JSON.
pub fn build_concordance(reports_dir: &Path) -> Result<Value, MifRhError> {
    let topics = list_topics(reports_dir)?;

    let mut all_concepts = Vec::new();
    let mut all_entities = Vec::new();
    let mut all_edges = Vec::new();
    for topic in &topics {
        let graph = build_topic_graph(reports_dir, topic)?;
        all_concepts.extend(graph.concepts);
        all_entities.extend(graph.entities);
        all_edges.extend(graph.edges);
    }

    let concepts = merge_nodes_by_id(all_concepts);
    let entities = merge_nodes_by_id(all_entities);
    let mut known: Vec<Value> = concepts;
    known.extend(entities);
    let known_ids: HashSet<String> = known
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    let edges = dedupe_edges(all_edges);
    let mut stub_targets: BTreeSet<String> = BTreeSet::new();
    for edge in &edges {
        if let Some(target) = edge.get("target").and_then(Value::as_str)
            && !known_ids.contains(target)
        {
            stub_targets.insert(target.to_string());
        }
    }
    let mut nodes = known;
    for target in &stub_targets {
        nodes.push(json!({
            "id": target, "kind": "concept", "label": target, "topics": [],
            "entityType": Value::Null, "ontology": Value::Null, "verdict": Value::Null,
            "flagged": false, "external": true,
        }));
    }
    nodes.sort_by(|a, b| {
        a.get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(b.get("id").and_then(Value::as_str).unwrap_or_default())
    });
    let mut edges = edges;
    edges.sort_by_key(edge_sort_key);

    Ok(json!({
        "@type": "Concordance",
        "generator": "build-concordance.sh (MIF-native ontological spine; SPEC §8d)",
        "nodes": nodes,
        "edges": edges,
    }))
}

#[cfg(test)]
mod tests {
    use super::build_concordance;
    use std::fs;

    fn write_finding(dir: &std::path::Path, topic: &str, name: &str, contents: &str) {
        let findings_dir = dir.join(topic).join("findings");
        fs::create_dir_all(&findings_dir).unwrap();
        fs::write(findings_dir.join(name), contents).unwrap();
    }

    #[test]
    fn builds_one_concept_node_per_finding_across_topics() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "topic-a",
            "f1.json",
            r#"{"@id": "urn:mif:f1", "title": "F1"}"#,
        );
        write_finding(
            dir.path(),
            "topic-b",
            "f2.json",
            r#"{"@id": "urn:mif:f2", "title": "F2"}"#,
        );

        let concordance = build_concordance(dir.path()).unwrap();
        let nodes = concordance["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn merges_an_entity_referenced_from_two_topics_unioning_their_topics() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "topic-a",
            "f1.json",
            r#"{"@id": "urn:mif:f1", "entities": [{"entity": {"@id": "urn:mif:entity:tool:widget"}, "name": "Widget"}]}"#,
        );
        write_finding(
            dir.path(),
            "topic-b",
            "f2.json",
            r#"{"@id": "urn:mif:f2", "entities": [{"entity": {"@id": "urn:mif:entity:tool:widget"}, "name": "Widget"}]}"#,
        );

        let concordance = build_concordance(dir.path()).unwrap();
        let entity = concordance["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "urn:mif:entity:tool:widget")
            .unwrap();
        assert_eq!(entity["topics"], serde_json::json!(["topic-a", "topic-b"]));
    }

    #[test]
    fn flags_a_falsified_finding_instead_of_excluding_it() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "topic-a",
            "f1.json",
            r#"{"@id": "urn:mif:f1", "extensions": {"harness": {"verification": {"verdict": "falsified"}}}}"#,
        );

        let concordance = build_concordance(dir.path()).unwrap();
        let nodes = concordance["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["flagged"], true);
        assert_eq!(nodes[0]["verdict"], "falsified");
    }

    #[test]
    fn materializes_an_external_stub_for_a_cross_topic_relationship_target() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "topic-a",
            "f1.json",
            r#"{"@id": "urn:mif:f1", "relationships": [{"target": "urn:mif:outside", "type": "supports"}]}"#,
        );

        let concordance = build_concordance(dir.path()).unwrap();
        let external: Vec<_> = concordance["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["external"] == true)
            .collect();
        assert_eq!(external.len(), 1);
        assert_eq!(external[0]["id"], "urn:mif:outside");
    }

    #[test]
    fn folds_in_ontology_map_entity_type_and_ontology() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(dir.path(), "topic-a", "f1.json", r#"{"@id": "urn:mif:f1"}"#);
        fs::write(
            dir.path().join("topic-a/ontology-map.json"),
            r#"[{"finding_id": "urn:mif:f1", "entity_type": "tool", "resolved_ontology": "software"}]"#,
        )
        .unwrap();

        let concordance = build_concordance(dir.path()).unwrap();
        let node = &concordance["nodes"][0];
        assert_eq!(node["entityType"], "tool");
        assert_eq!(node["ontology"], "software");
    }

    #[test]
    fn nodes_are_sorted_by_id() {
        let dir = tempfile::tempdir().unwrap();
        write_finding(
            dir.path(),
            "topic-a",
            "f1.json",
            r#"{"@id": "urn:mif:zzz"}"#,
        );
        write_finding(
            dir.path(),
            "topic-a",
            "f2.json",
            r#"{"@id": "urn:mif:aaa"}"#,
        );

        let concordance = build_concordance(dir.path()).unwrap();
        let ids: Vec<&str> = concordance["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["urn:mif:aaa", "urn:mif:zzz"]);
    }
}
