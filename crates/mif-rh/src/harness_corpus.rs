//! Cross-topic corpus atlas (rht Category B, Story #282; Epic 2,
//! ontological spine, ADR-0011).
//!
//! Ports rht's `scripts/synthesize-corpus.sh`: projects
//! `reports/concordance.json` into a corpus-level view spanning every
//! topic, including what was falsified/weakened (the per-topic
//! report-synthesizer deliberately keeps survivors only). All structure
//! comes from the already-merged concordance, so this never opens a
//! finding file.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::error::MifRhError;
use crate::harness_project::read_json;

/// Rows highlighted in the "Entity Reuse" table (matches the bash
/// script's fixed `HIGHLIGHTS=12`).
const HIGHLIGHT_COUNT: usize = 12;

/// The result of a [`synthesize_corpus`] call.
#[derive(Debug, Clone)]
pub struct CorpusSynthesis {
    /// The full `corpus-map.json` content (key-sorted by the caller
    /// before writing, matching the original's `jq -S`).
    pub map: Value,
    /// The full `corpus-synthesis.md` content.
    pub markdown: String,
    /// Topic count (for the CLI's summary line).
    pub topics: usize,
    /// Cross-topic entity count (for the CLI's summary line).
    pub entities: usize,
}

fn is_contradiction_type(edge_type: &str) -> bool {
    edge_type.contains("contradict") || edge_type.contains("refut") || edge_type.contains("disput")
}

fn node_topics(node: &Value) -> Vec<String> {
    let mut topics: Vec<String> = node
        .get("topics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect();
    topics.sort();
    topics
}

fn build_entity_reuse(entities: &[&Value], edges: &[&Value]) -> Vec<Value> {
    let mut entity_reuse: Vec<Value> = entities
        .iter()
        .map(|entity| {
            let id = entity
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let entity_topics = node_topics(entity);
            let degree = edges
                .iter()
                .filter(|e| {
                    e.get("source").and_then(Value::as_str) == Some(id.as_str())
                        || e.get("target").and_then(Value::as_str) == Some(id.as_str())
                })
                .count();
            json!({
                "id": id,
                "label": entity.get("label").cloned().unwrap_or(Value::Null),
                "entityType": entity.get("entityType").cloned().unwrap_or(Value::Null),
                "topic_count": entity_topics.len(),
                "topics": entity_topics,
                "degree": degree,
            })
        })
        .collect();
    entity_reuse.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                std::cmp::Reverse(v["topic_count"].as_u64().unwrap_or(0)),
                std::cmp::Reverse(v["degree"].as_u64().unwrap_or(0)),
                v["id"].as_str().unwrap_or_default().to_string(),
            )
        };
        key(a).cmp(&key(b))
    });
    entity_reuse
}

fn build_contradictions(edges: &[&Value]) -> Vec<Value> {
    let mut contradictions: Vec<Value> = edges
        .iter()
        .filter(|e| {
            e.get("via").and_then(Value::as_str) == Some("relationship")
                && e.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(is_contradiction_type)
        })
        .map(|e| {
            json!({
                "source": e.get("source").cloned().unwrap_or(Value::Null),
                "target": e.get("target").cloned().unwrap_or(Value::Null),
                "type": e.get("type").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    contradictions.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["source"].as_str().unwrap_or_default().to_string(),
                v["target"].as_str().unwrap_or_default().to_string(),
                v["type"].as_str().unwrap_or_default().to_string(),
            )
        };
        key(a).cmp(&key(b))
    });
    contradictions
}

fn build_disproven(concepts: &[&&Value]) -> Vec<Value> {
    let mut disproven: Vec<Value> = concepts
        .iter()
        .filter(|c| c.get("flagged").and_then(Value::as_bool) == Some(true))
        .map(|c| {
            json!({
                "id": c.get("id").cloned().unwrap_or(Value::Null),
                "label": c.get("label").cloned().unwrap_or(Value::Null),
                "topics": node_topics(c),
            })
        })
        .collect();
    disproven.sort_by(|a, b| {
        a["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["id"].as_str().unwrap_or_default())
    });
    disproven
}

fn build_corpus_map(concordance: &Value) -> Value {
    let nodes: Vec<&Value> = concordance
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let edges: Vec<&Value> = concordance
        .get("edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let concepts: Vec<&&Value> = nodes
        .iter()
        .filter(|n| n.get("kind").and_then(Value::as_str) == Some("concept"))
        .collect();
    let entities: Vec<&Value> = nodes
        .iter()
        .filter(|n| n.get("kind").and_then(Value::as_str) == Some("entity"))
        .copied()
        .collect();

    let mut topics: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for node in &nodes {
        topics.extend(node_topics(node));
    }

    let mut verdict_distribution: BTreeMap<String, usize> = BTreeMap::new();
    for concept in &concepts {
        let verdict = concept
            .get("verdict")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_string();
        *verdict_distribution.entry(verdict).or_insert(0) += 1;
    }

    let entity_reuse = build_entity_reuse(&entities, &edges);
    let contradictions = build_contradictions(&edges);
    let disproven = build_disproven(&concepts);

    json!({
        "@type": "CorpusMap",
        "generator": "synthesize-corpus.sh",
        "topics": topics.into_iter().collect::<Vec<_>>(),
        "verdict_distribution": verdict_distribution,
        "entity_reuse": entity_reuse,
        "contradictions": contradictions,
        "disproven": disproven,
    })
}

fn render_header(map: &Value) -> String {
    use std::fmt::Write as _;

    let topics = map["topics"].as_array().map_or(0, Vec::len);
    let entities = map["entity_reuse"].as_array().map_or(0, Vec::len);
    let verdict = |name: &str| {
        map["verdict_distribution"]
            .get(name)
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let (surv, weak, inc, fals) = (
        verdict("survived"),
        verdict("weakened"),
        verdict("inconclusive"),
        verdict("falsified"),
    );
    let total_findings: u64 = map["verdict_distribution"]
        .as_object()
        .map_or(0, |o| o.values().filter_map(Value::as_u64).sum());

    let mut out = String::new();
    out.push_str("# Corpus Atlas\n\n");
    let _ = write!(
        out,
        "**Topics:** {topics} | **Findings:** {total_findings} (survived {surv}, weakened {weak}, inconclusive {inc}, falsified {fals}) | **Entities:** {entities}\n\n"
    );
    out.push_str("The cross-topic ontological spine as a single view: what the whole corpus knows, including what was disproven. Unlike a per-topic report (survivors only), this atlas keeps the entire research record.\n\n");
    out.push_str("---\n\n");
    out
}

fn render_insights_section(preserved_insights: Option<&str>) -> String {
    use std::fmt::Write as _;

    let mut out = String::from("## Cross-Corpus Insights\n\n");
    match preserved_insights {
        Some(text) if !text.is_empty() => {
            let _ = write!(out, "{text}\n\n");
        },
        _ => out.push_str(
            "- _Draft — the corpus-synthesizer replaces this with cross-topic synthesis (entity reuse, converging vs. contradicting evidence, and what was disproven)._\n\n",
        ),
    }
    out
}

fn render_entity_reuse_section(map: &Value) -> String {
    use std::fmt::Write as _;

    let mut out = String::from("## Entity Reuse\n\n");
    let entity_rows = map["entity_reuse"].as_array().cloned().unwrap_or_default();
    if entity_rows.is_empty() {
        out.push_str("No cross-topic entities yet.\n\n");
    } else {
        out.push_str(
            "| Entity | Type | Topics | Cross-topic | Degree |\n| --- | --- | --- | --- | --- |\n",
        );
        for entity in entity_rows.iter().take(HIGHLIGHT_COUNT) {
            let label = entity
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let entity_type = entity
                .get("entityType")
                .and_then(Value::as_str)
                .unwrap_or("—");
            let entity_topics: Vec<&str> = entity["topics"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let topic_count = entity
                .get("topic_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let degree = entity.get("degree").and_then(Value::as_u64).unwrap_or(0);
            let _ = writeln!(
                out,
                "| {label} | {entity_type} | {} | {topic_count} | {degree} |",
                entity_topics.join(", ")
            );
        }
        out.push('\n');
    }
    out
}

fn render_contradictions_section(map: &Value) -> String {
    use std::fmt::Write as _;

    let mut out = String::from("## Contradictions\n\n");
    let contradiction_rows = map["contradictions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if contradiction_rows.is_empty() {
        out.push_str("No cross-topic contradictions recorded.\n\n");
    } else {
        for row in &contradiction_rows {
            let source = row
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let edge_type = row.get("type").and_then(Value::as_str).unwrap_or_default();
            let target = row
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let _ = writeln!(out, "- `{source}` —{edge_type}→ `{target}`");
        }
        out.push('\n');
    }
    out
}

fn render_disproven_section(map: &Value) -> String {
    use std::fmt::Write as _;

    let mut out = String::from("## What Was Disproven\n\n");
    let disproven_rows = map["disproven"].as_array().cloned().unwrap_or_default();
    if disproven_rows.is_empty() {
        out.push_str("No findings were falsified.\n\n");
    } else {
        for row in &disproven_rows {
            let label = row.get("label").and_then(Value::as_str).unwrap_or_default();
            let row_topics: Vec<&str> = row["topics"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let _ = writeln!(out, "- {label} _(topics: {})_", row_topics.join(", "));
        }
        out.push('\n');
    }
    out
}

fn render_topics_section(map: &Value) -> String {
    use std::fmt::Write as _;

    let mut out = String::from("## Topics\n\n");
    let topic_list: Vec<&str> = map["topics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if topic_list.is_empty() {
        out.push_str("—\n");
    } else {
        for topic in &topic_list {
            let _ = writeln!(out, "- {topic}");
        }
    }
    out
}

fn render_markdown(map: &Value, preserved_insights: Option<&str>) -> String {
    let mut out = render_header(map);
    out.push_str(&render_insights_section(preserved_insights));
    out.push_str(&render_entity_reuse_section(map));
    out.push_str(&render_contradictions_section(map));
    out.push_str(&render_disproven_section(map));
    out.push_str(&render_topics_section(map));
    out
}

/// Builds the cross-topic corpus atlas from `concordance_path`
/// (`reports/concordance.json`).
///
/// # Errors
///
/// Returns [`MifRhError::Io`]/[`MifRhError::Json`] if the concordance
/// cannot be read/parsed, and [`MifRhError::InvalidConcordance`] if it
/// parses but is not an object with a `nodes` array.
pub fn synthesize_corpus(
    concordance_path: &Path,
    preserved_insights: Option<&str>,
) -> Result<CorpusSynthesis, MifRhError> {
    let concordance = read_json(concordance_path)?;
    if !concordance.is_object() || !concordance.get("nodes").is_some_and(Value::is_array) {
        return Err(MifRhError::InvalidConcordance {
            path: concordance_path.display().to_string(),
        });
    }

    let map = build_corpus_map(&concordance);
    let markdown = render_markdown(&map, preserved_insights);
    let topics = map["topics"].as_array().map_or(0, Vec::len);
    let entities = map["entity_reuse"].as_array().map_or(0, Vec::len);

    Ok(CorpusSynthesis {
        map,
        markdown,
        topics,
        entities,
    })
}

#[cfg(test)]
mod tests {
    use super::synthesize_corpus;
    use std::fs;

    fn write_concordance(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("concordance.json");
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn errors_when_concordance_has_no_nodes_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_concordance(dir.path(), r#"{"edges": []}"#);

        let error = synthesize_corpus(&path, None).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::InvalidConcordance { .. }
        ));
    }

    #[test]
    fn projects_topics_and_verdict_distribution() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_concordance(
            dir.path(),
            r#"{"nodes": [
                {"id": "a", "kind": "concept", "verdict": "survived", "topics": ["t1"], "flagged": false},
                {"id": "b", "kind": "concept", "verdict": "falsified", "topics": ["t2"], "flagged": true}
            ], "edges": []}"#,
        );

        let synthesis = synthesize_corpus(&path, None).unwrap();
        assert_eq!(synthesis.topics, 2);
        assert_eq!(synthesis.map["verdict_distribution"]["survived"], 1);
        assert_eq!(synthesis.map["verdict_distribution"]["falsified"], 1);
        assert_eq!(synthesis.map["disproven"].as_array().unwrap().len(), 1);
        assert_eq!(synthesis.map["disproven"][0]["id"], "b");
    }

    #[test]
    fn ranks_entity_reuse_by_topic_count_then_degree_then_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_concordance(
            dir.path(),
            r#"{"nodes": [
                {"id": "e1", "kind": "entity", "label": "E1", "topics": ["t1"]},
                {"id": "e2", "kind": "entity", "label": "E2", "topics": ["t1", "t2"]}
            ], "edges": [
                {"source": "x", "target": "e1", "via": "entity"},
                {"source": "x", "target": "e2", "via": "entity"},
                {"source": "y", "target": "e2", "via": "entity"}
            ]}"#,
        );

        let synthesis = synthesize_corpus(&path, None).unwrap();
        let reuse = synthesis.map["entity_reuse"].as_array().unwrap();
        assert_eq!(reuse[0]["id"], "e2");
        assert_eq!(reuse[0]["topic_count"], 2);
        assert_eq!(reuse[0]["degree"], 2);
        assert_eq!(reuse[1]["id"], "e1");
    }

    #[test]
    fn detects_a_contradiction_edge_by_type_substring() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_concordance(
            dir.path(),
            r#"{"nodes": [], "edges": [
                {"source": "a", "target": "b", "via": "relationship", "type": "contradicts"},
                {"source": "a", "target": "c", "via": "relationship", "type": "supports"}
            ]}"#,
        );

        let synthesis = synthesize_corpus(&path, None).unwrap();
        let contradictions = synthesis.map["contradictions"].as_array().unwrap();
        assert_eq!(contradictions.len(), 1);
        assert_eq!(contradictions[0]["target"], "b");
    }

    #[test]
    fn markdown_uses_the_draft_marker_when_no_prose_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_concordance(dir.path(), r#"{"nodes": [], "edges": []}"#);

        let synthesis = synthesize_corpus(&path, None).unwrap();
        assert!(
            synthesis
                .markdown
                .contains("_Draft — the corpus-synthesizer")
        );
    }

    #[test]
    fn markdown_uses_preserved_prose_when_given() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_concordance(dir.path(), r#"{"nodes": [], "edges": []}"#);

        let synthesis = synthesize_corpus(&path, Some("Authored cross-topic synthesis.")).unwrap();
        assert!(
            synthesis
                .markdown
                .contains("Authored cross-topic synthesis.")
        );
        assert!(!synthesis.markdown.contains("_Draft —"));
    }
}
