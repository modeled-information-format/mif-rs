//! Knowledge-graph MIF-substrate assertion (rht Category B, Story #287).
//!
//! Ports rht's `scripts/assert-graph-mif.sh`: proves a `knowledge-graph.json`
//! is built from MIF entities and typed relationships, not tag co-occurrence
//! (Milestone 4 acceptance gate).

use std::path::Path;

use serde_json::Value;

use crate::error::MifRhError;
use crate::harness_project::read_json;

/// One named boolean assertion and whether it held.
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// The human-readable description of what was checked (matches the
    /// original script's per-check message).
    pub message: String,
    /// Whether the check passed.
    pub passed: bool,
}

/// The full result of an [`assert_graph_mif`] run: every individual check,
/// in the same fixed order as the original script, plus the overall verdict.
#[derive(Debug, Clone)]
pub struct GraphAssertion {
    /// Every check, in order.
    pub checks: Vec<CheckResult>,
    /// Whether every check passed.
    pub passed: bool,
}

fn is_urn_mif(value: &Value) -> bool {
    value.as_str().is_some_and(|s| s.starts_with("urn:mif:"))
}

fn check(message: &str, passed: bool) -> CheckResult {
    CheckResult {
        message: message.to_string(),
        passed,
    }
}

/// Proves `graph`'s nodes/edges are MIF-derived.
///
/// Every id is a `urn:mif:` identifier, at least one edge is a typed
/// relationship, every referenced MIF entity has a corresponding entity node
/// (when any entity-mention edge exists), and every edge target resolves to
/// a real node.
#[must_use]
pub fn assert_graph_mif(graph: &Value) -> GraphAssertion {
    let nodes: Vec<&Value> = graph
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let edges: Vec<&Value> = graph
        .get("edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();

    let node_ids: std::collections::HashSet<&str> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();

    let entity_node_ids: std::collections::HashSet<&str> = nodes
        .iter()
        .filter(|n| n.get("kind").and_then(Value::as_str) == Some("entity"))
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .filter(|id| id.starts_with("urn:mif:entity:"))
        .collect();
    // Stricter than the original bash+jq check this ports, which only asked
    // "does ANY conforming entity node exist anywhere" — that let some
    // entity-mention edges point at missing/non-entity targets as long as
    // one other edge's target happened to resolve. Checking EVERY
    // entity-mention edge's OWN target only tightens the gate: any graph
    // that passed the old check still passes this one.
    let every_entity_edge_resolves = edges
        .iter()
        .filter(|e| e.get("via").and_then(Value::as_str) == Some("entity"))
        .all(|e| {
            e.get("target")
                .and_then(Value::as_str)
                .is_some_and(|t| entity_node_ids.contains(t))
        });

    let checks = vec![
        check("graph has nodes", !nodes.is_empty()),
        check("graph has edges", !edges.is_empty()),
        check(
            "every node id is a urn:mif: identifier (not a tag)",
            nodes.iter().all(|n| is_urn_mif(&n["id"])),
        ),
        check(
            "every edge source is a urn:mif: concept",
            edges.iter().all(|e| is_urn_mif(&e["source"])),
        ),
        check(
            "every edge target is a urn:mif: id",
            edges.iter().all(|e| is_urn_mif(&e["target"])),
        ),
        check(
            "at least one edge derives from a typed MIF relationship",
            edges
                .iter()
                .any(|e| e.get("via").and_then(Value::as_str) == Some("relationship")),
        ),
        check(
            "every referenced MIF entity is an entity node",
            every_entity_edge_resolves,
        ),
        check(
            "every edge target resolves to a node in the graph",
            edges.iter().all(|e| {
                e.get("target")
                    .and_then(Value::as_str)
                    .is_some_and(|t| node_ids.contains(t))
            }),
        ),
    ];

    let passed = checks.iter().all(|c| c.passed);
    GraphAssertion { checks, passed }
}

/// Reads `graph_path` as JSON and runs [`assert_graph_mif`] against it.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `graph_path` cannot be read, and
/// [`MifRhError::Json`] if it is not valid JSON.
pub fn assert_graph_mif_file(graph_path: &Path) -> Result<GraphAssertion, MifRhError> {
    let graph = read_json(graph_path)?;
    Ok(assert_graph_mif(&graph))
}

#[cfg(test)]
mod tests {
    use super::assert_graph_mif;
    use serde_json::json;

    #[test]
    fn passes_a_well_formed_mif_graph() {
        let graph = json!({
            "nodes": [
                {"id": "urn:mif:f1", "kind": "concept"},
                {"id": "urn:mif:entity:e1", "kind": "entity"}
            ],
            "edges": [
                {"source": "urn:mif:f1", "target": "urn:mif:f1", "via": "relationship"},
                {"source": "urn:mif:f1", "target": "urn:mif:entity:e1", "via": "entity"}
            ]
        });

        let assertion = assert_graph_mif(&graph);
        assert!(assertion.passed, "{:?}", assertion.checks);
    }

    #[test]
    fn fails_when_a_node_id_is_a_bare_tag() {
        let graph = json!({
            "nodes": [{"id": "some-tag", "kind": "concept"}],
            "edges": [{"source": "some-tag", "target": "some-tag", "via": "relationship"}]
        });

        let assertion = assert_graph_mif(&graph);
        assert!(!assertion.passed);
        assert!(
            !assertion.checks[2].passed,
            "node-id-is-urn check should fail"
        );
    }

    #[test]
    fn fails_when_no_edge_is_a_typed_relationship() {
        let graph = json!({
            "nodes": [{"id": "urn:mif:f1", "kind": "concept"}],
            "edges": [{"source": "urn:mif:f1", "target": "urn:mif:f1", "via": "entity"}]
        });

        let assertion = assert_graph_mif(&graph);
        assert!(!assertion.checks[5].passed);
    }

    #[test]
    fn requires_an_entity_node_only_when_an_entity_mention_edge_exists() {
        // No entity-mention edges at all: the entity-node check is
        // vacuously satisfied even with zero entity nodes.
        let graph = json!({
            "nodes": [{"id": "urn:mif:f1", "kind": "concept"}],
            "edges": [{"source": "urn:mif:f1", "target": "urn:mif:f1", "via": "relationship"}]
        });
        let assertion = assert_graph_mif(&graph);
        assert!(assertion.checks[6].passed);
    }

    #[test]
    fn fails_when_an_entity_mention_edge_has_no_matching_entity_node() {
        let graph = json!({
            "nodes": [{"id": "urn:mif:f1", "kind": "concept"}],
            "edges": [
                {"source": "urn:mif:f1", "target": "urn:mif:f1", "via": "relationship"},
                {"source": "urn:mif:f1", "target": "urn:mif:entity:missing", "via": "entity"}
            ]
        });
        let assertion = assert_graph_mif(&graph);
        assert!(!assertion.checks[6].passed);
    }

    #[test]
    fn fails_when_one_of_several_entity_mention_edges_targets_a_non_entity_node() {
        // A valid entity node exists (satisfying a loose "does any entity node
        // exist" check), but a SECOND entity-mention edge targets a node that
        // is not an entity at all — this must still fail, not pass on the
        // strength of the first edge's valid target.
        let graph = json!({
            "nodes": [
                {"id": "urn:mif:f1", "kind": "concept"},
                {"id": "urn:mif:entity:acme", "kind": "entity"}
            ],
            "edges": [
                {"source": "urn:mif:f1", "target": "urn:mif:entity:acme", "via": "entity"},
                {"source": "urn:mif:f1", "target": "urn:mif:f1", "via": "entity"}
            ]
        });
        let assertion = assert_graph_mif(&graph);
        assert!(!assertion.checks[6].passed);
    }

    #[test]
    fn fails_when_an_edge_target_does_not_resolve_to_a_node() {
        let graph = json!({
            "nodes": [{"id": "urn:mif:f1", "kind": "concept"}],
            "edges": [
                {"source": "urn:mif:f1", "target": "urn:mif:f1", "via": "relationship"},
                {"source": "urn:mif:f1", "target": "urn:mif:ghost", "via": "relationship"}
            ]
        });
        let assertion = assert_graph_mif(&graph);
        assert!(!assertion.checks[7].passed);
    }

    #[test]
    fn fails_closed_on_an_empty_graph() {
        let graph = json!({"nodes": [], "edges": []});
        let assertion = assert_graph_mif(&graph);
        assert!(!assertion.passed);
        assert!(!assertion.checks[0].passed);
        assert!(!assertion.checks[1].passed);
    }
}
