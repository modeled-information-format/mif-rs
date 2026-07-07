//! Fail-closed ontology conformance for the concordance (rht Category B,
//! Story #287, SPEC §8d).
//!
//! Ports rht's `scripts/validate-concordance.sh`: asserts that every node's
//! `entityType` and every relationship edge's type is declared by an
//! ontology bound to the node's topic(s) (core ∪ the topic's bound
//! ontologies ∪ their transitive `extends` ancestors), and that each
//! relationship edge's endpoints satisfy the declared relationship's
//! `from`/`to` domains — a finer subtype (via `subtype_of`) satisfies a
//! domain written against any of its transitive supertypes
//! (Liskov substitutability). Mention edges (`via: "entity"`) are
//! structural and never domain-checked.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use crate::catalog::Catalog;
use crate::config::HarnessConfig;
use crate::error::MifRhError;
use crate::ontology_pack::{OntologyPack, RelationshipDecl};
use crate::resolve::{ResolveContext, build_allowed};

/// The MIF spec's core entity-type vocabulary (`mif_core::KnownEntityType`'s
/// variants, as they appear in `schemas/mif/definitions/entity-reference.schema.json`'s
/// enum) — always valid, regardless of topic bindings.
const MIF_CORE_TYPES: [&str; 5] = ["Concept", "File", "Organization", "Person", "Technology"];

/// MIF-native structural relationship types, always domain-check-exempt.
/// Unioned with any relationship type a CORE ontology itself declares (so a
/// core ontology like `mif-base` can add to this set without the harness
/// hard-coding its name here too).
const STRUCTURAL_CORE: [&str; 9] = [
    "supports",
    "contradicts",
    "derived-from",
    "relates-to",
    "supersedes",
    "refines",
    "part-of",
    "depends-on",
    "updates",
];

/// One [`validate_concordance`] run's result.
#[derive(Debug, Clone)]
pub struct ConcordanceValidation {
    /// Every conformance violation found, in node-then-edge order.
    pub violations: Vec<String>,
    /// The concordance's node count (for the summary line).
    pub nodes: usize,
    /// The concordance's edge count (for the summary line).
    pub edges: usize,
}

impl ConcordanceValidation {
    /// Whether the concordance is fully conformant (no violations).
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Cycle-safe transitive supertype walk: `t` plus every ancestor reachable
/// via `parents` (a diamond — a shared ancestor via two paths — is fine;
/// `seen` is the current path, not the visited set, so only a true cycle
/// (`t` revisiting itself on its own path) errors).
fn supers<'a>(
    t: &'a str,
    parents: &HashMap<&'a str, &'a [String]>,
    seen: &mut Vec<&'a str>,
) -> Result<Vec<String>, MifRhError> {
    if seen.contains(&t) {
        return Err(MifRhError::SubtypeOfCycle {
            entity_type: t.to_string(),
        });
    }
    seen.push(t);
    let mut out = vec![t.to_string()];
    for parent in parents.get(t).copied().unwrap_or_default() {
        for ancestor in supers(parent, parents, seen)? {
            if !out.contains(&ancestor) {
                out.push(ancestor);
            }
        }
    }
    seen.pop();
    Ok(out)
}

/// Cycle-detecting transitive `subtype_of` closure: every declared entity
/// type name maps to itself plus every transitive ancestor, across every
/// pack in `packs` (a subtype's supertypes may be declared in a different
/// pack than the subtype itself, so this always closes over the whole set,
/// not one pack at a time).
///
/// # Errors
///
/// Returns [`MifRhError::SubtypeOfCycle`] if a type's `subtype_of` chain
/// revisits itself.
fn supertype_closure(packs: &[&OntologyPack]) -> Result<HashMap<String, Vec<String>>, MifRhError> {
    let mut parents: HashMap<&str, &[String]> = HashMap::new();
    let mut all_types: HashSet<&str> = HashSet::new();
    for pack in packs {
        for entity_type in &pack.entity_types {
            all_types.insert(entity_type.name.as_str());
            if !entity_type.subtype_of.is_empty() {
                parents.insert(entity_type.name.as_str(), &entity_type.subtype_of);
            }
        }
    }

    let mut closure = HashMap::new();
    for t in &all_types {
        let mut seen = Vec::new();
        closure.insert((*t).to_string(), supers(t, &parents, &mut seen)?);
    }
    Ok(closure)
}

/// Whether entity type `entity_type` (or any of its transitive supertypes)
/// appears in `domain`.
fn hits(entity_type: &str, domain: &[String], supertypes: &HashMap<String, Vec<String>>) -> bool {
    supertypes.get(entity_type).map_or_else(
        || domain.iter().any(|d| d == entity_type),
        |ancestry| ancestry.iter().any(|a| domain.contains(a)),
    )
}

/// The union of packs allowed for any of `topics` (deduplicated by id),
/// looking up each topic in `allowed_by_topic` (absent -> no packs for that
/// topic, matching the original script's `$allowed[$t] // []`).
fn allowed_for_topics<'a>(
    allowed_by_topic: &HashMap<String, Vec<&'a OntologyPack>>,
    topics: &[String],
) -> Vec<&'a OntologyPack> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for topic in topics {
        for pack in allowed_by_topic.get(topic).into_iter().flatten() {
            if seen.insert(pack.id.as_str()) {
                out.push(*pack);
            }
        }
    }
    out
}

fn node_topics(node: &Value) -> Vec<String> {
    node.get("topics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect()
}

/// Per-topic allowed ontology packs, and the union of every pack relevant
/// to any topic (for the supertype closure and the core-ontology
/// STRUCTURAL scan).
type AllowedByTopic<'a> = (
    HashMap<String, Vec<&'a OntologyPack>>,
    HashMap<&'a str, &'a OntologyPack>,
);

/// Builds the per-topic allowed-ontology-pack map, and the union of every
/// pack relevant to any topic (for the supertype closure and the
/// core-ontology STRUCTURAL scan).
///
/// # Errors
///
/// Returns [`MifRhError::DirectBindingInvalid`] if a topic binds an
/// uncataloged or version-mismatched ontology, or [`MifRhError::Ontology`]
/// if an ontology's `extends` chain cannot be resolved.
fn build_allowed_by_topic<'a>(
    config: &'a HarnessConfig,
    catalog: &'a Catalog,
    ontology_packs: &'a HashMap<String, OntologyPack>,
) -> Result<AllowedByTopic<'a>, MifRhError> {
    let mut allowed_by_topic = HashMap::new();
    let mut relevant: HashMap<&str, &OntologyPack> = HashMap::new();
    for topic in &config.topics {
        let ctx = ResolveContext {
            topic: &topic.id,
            catalog,
            config,
            ontology_packs,
        };
        let allowed = build_allowed(&ctx)?;
        for pack in &allowed {
            relevant.insert(pack.id.as_str(), pack);
        }
        allowed_by_topic.insert(topic.id.clone(), allowed);
    }
    Ok((allowed_by_topic, relevant))
}

fn structural_relationship_types(
    catalog: &Catalog,
    relevant: &HashMap<&str, &OntologyPack>,
) -> HashSet<String> {
    STRUCTURAL_CORE
        .iter()
        .map(|s| (*s).to_string())
        .chain(catalog.core_ids().flat_map(|id| {
            relevant
                .get(id)
                .into_iter()
                .flat_map(|pack| pack.relationships.keys().cloned())
        }))
        .collect()
}

fn check_node(
    node: &Value,
    allowed_by_topic: &HashMap<String, Vec<&OntologyPack>>,
) -> Option<String> {
    let entity_type = node.get("entityType").and_then(Value::as_str)?;
    if node.get("external").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let topics = node_topics(node);
    let allowed = allowed_for_topics(allowed_by_topic, &topics);
    let declared = MIF_CORE_TYPES.contains(&entity_type)
        || allowed
            .iter()
            .any(|pack| pack.entity_types.iter().any(|et| et.name == entity_type));
    if declared {
        return None;
    }
    let id = node.get("id").and_then(Value::as_str).unwrap_or("");
    let topic_label = topics.first().map_or("<id>", String::as_str);
    Some(format!(
        "node {id} (topic: {}): entityType {entity_type} not in MIF core nor declared by a \
         bound ontology — fix: /ontology-review --topic {topic_label} --enrich",
        topics.join(",")
    ))
}

#[allow(clippy::too_many_arguments)]
fn check_edge(
    edge: &Value,
    by_id: &HashMap<&str, &Value>,
    allowed_by_topic: &HashMap<String, Vec<&OntologyPack>>,
    structural: &HashSet<String>,
    supertypes: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if edge.get("via").and_then(Value::as_str) != Some("relationship") {
        return None;
    }
    let source_id = edge.get("source").and_then(Value::as_str).unwrap_or("");
    let target_id = edge.get("target").and_then(Value::as_str).unwrap_or("");
    let edge_type = edge.get("type").and_then(Value::as_str).unwrap_or("");
    if structural.contains(edge_type) {
        return None;
    }
    let source = by_id.get(source_id).copied();
    let target = by_id.get(target_id).copied();
    let source_topics = source.map(node_topics).unwrap_or_default();
    let source_et = source
        .and_then(|n| n.get("entityType"))
        .and_then(Value::as_str);
    let target_et = target
        .and_then(|n| n.get("entityType"))
        .and_then(Value::as_str);

    let allowed = allowed_for_topics(allowed_by_topic, &source_topics);
    let rels: Vec<&RelationshipDecl> = allowed
        .iter()
        .filter_map(|pack| pack.relationships.get(edge_type))
        .collect();

    if rels.is_empty() {
        return Some(format!(
            "edge {source_id} ->{edge_type}-> {target_id} (topic: {}): relationship type not \
             MIF-core nor declared by a bound ontology — fix: /ontology-review --topic {} \
             --enrich",
            source_topics.join(","),
            source_topics.first().map_or("<id>", String::as_str)
        ));
    }
    let source_et_str = source_et.unwrap_or("");
    let target_et_str = target_et.unwrap_or("");
    let satisfied = rels.iter().any(|rel| {
        hits(source_et_str, &rel.from, supertypes) && hits(target_et_str, &rel.to, supertypes)
    });
    if satisfied {
        return None;
    }
    Some(format!(
        "edge {source_id} ->{edge_type}-> {target_id}: from/to domain violation ({} -> {})",
        source_et.unwrap_or("null"),
        target_et.unwrap_or("null")
    ))
}

/// Validates every node's `entityType` and every relationship edge's type
/// in `concordance` against the ontologies bound to each node's topic(s).
///
/// `root` resolves each cataloged ontology's `source` path.
///
/// # Errors
///
/// Returns [`MifRhError::DirectBindingInvalid`] if a topic binds an
/// uncataloged or version-mismatched ontology, [`MifRhError::Ontology`] if
/// an ontology's `extends` chain cannot be resolved (a missing ancestor or
/// a cycle), or [`MifRhError::SubtypeOfCycle`] if a `subtype_of` chain
/// revisits itself.
pub fn validate_concordance(
    concordance: &Value,
    config: &HarnessConfig,
    catalog: &Catalog,
    root: &Path,
) -> Result<ConcordanceValidation, MifRhError> {
    let ontology_packs = crate::ontology_pack::load_packs_via_catalog(catalog, root)?;
    let (allowed_by_topic, relevant) = build_allowed_by_topic(config, catalog, &ontology_packs)?;
    let relevant_packs: Vec<&OntologyPack> = relevant.values().copied().collect();
    let supertypes = supertype_closure(&relevant_packs)?;
    let structural = structural_relationship_types(catalog, &relevant);

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
    let by_id: HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str).map(|id| (id, *n)))
        .collect();

    let mut violations: Vec<String> = nodes
        .iter()
        .filter_map(|node| check_node(node, &allowed_by_topic))
        .collect();
    for edge in &edges {
        if let Some(violation) =
            check_edge(edge, &by_id, &allowed_by_topic, &structural, &supertypes)
        {
            violations.push(violation);
        }
    }

    Ok(ConcordanceValidation {
        violations,
        nodes: nodes.len(),
        edges: edges.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::validate_concordance;
    use crate::catalog::{Catalog, CatalogEntry};
    use crate::config::{HarnessConfig, TopicConfig};
    use serde_json::json;
    use std::fs;

    fn write(dir: &std::path::Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    const CORE_YAML: &str = "
ontology:
  id: mif-generic
  version: \"1.0.0\"
";

    fn catalog_with(entries: Vec<CatalogEntry>) -> Catalog {
        Catalog {
            ontologies: entries,
        }
    }

    #[test]
    fn passes_a_node_typed_by_a_bound_ontology() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        write(
            dir.path(),
            "edu-fixture.yaml",
            "
ontology:
  id: edu-fixture
  version: \"0.1.0\"
entity_types:
  - name: title
",
        );
        let catalog = catalog_with(vec![
            CatalogEntry {
                id: "mif-generic".to_string(),
                version: "1.0.0".to_string(),
                source: Some("core/mif-generic.yaml".to_string()),
                core: true,
            },
            CatalogEntry {
                id: "edu-fixture".to_string(),
                version: "0.1.0".to_string(),
                source: Some("edu-fixture.yaml".to_string()),
                core: false,
            },
        ]);
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "edu".to_string(),
                ontologies: vec!["edu-fixture".to_string()],
            }],
        };
        let concordance = json!({
            "nodes": [{"id": "n1", "entityType": "title", "topics": ["edu"]}],
            "edges": []
        });

        let result = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap();
        assert!(result.ok(), "{:?}", result.violations);
        assert_eq!(result.nodes, 1);
    }

    #[test]
    fn flags_an_undeclared_entity_type() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        let catalog = catalog_with(vec![CatalogEntry {
            id: "mif-generic".to_string(),
            version: "1.0.0".to_string(),
            source: Some("core/mif-generic.yaml".to_string()),
            core: true,
        }]);
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "edu".to_string(),
                ontologies: vec![],
            }],
        };
        let concordance = json!({
            "nodes": [{"id": "n1", "entityType": "title", "topics": ["edu"]}],
            "edges": []
        });

        let result = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap();
        assert!(!result.ok());
        assert!(result.violations[0].contains("entityType title not in MIF core"));
        assert!(result.violations[0].contains("/ontology-review --topic edu"));
    }

    #[test]
    fn a_mif_core_type_always_passes_even_with_no_bound_ontology() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        let catalog = catalog_with(vec![CatalogEntry {
            id: "mif-generic".to_string(),
            version: "1.0.0".to_string(),
            source: Some("core/mif-generic.yaml".to_string()),
            core: true,
        }]);
        let config = HarnessConfig { topics: vec![] };
        let concordance = json!({
            "nodes": [{"id": "n1", "entityType": "Organization", "topics": []}],
            "edges": []
        });

        let result = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap();
        assert!(result.ok(), "{:?}", result.violations);
    }

    #[test]
    fn an_external_stub_node_is_never_type_checked() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        let catalog = catalog_with(vec![CatalogEntry {
            id: "mif-generic".to_string(),
            version: "1.0.0".to_string(),
            source: Some("core/mif-generic.yaml".to_string()),
            core: true,
        }]);
        let config = HarnessConfig { topics: vec![] };
        let concordance = json!({
            "nodes": [{"id": "n1", "entityType": "nonsense-type", "external": true, "topics": []}],
            "edges": []
        });

        let result = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap();
        assert!(result.ok(), "{:?}", result.violations);
    }

    #[test]
    fn a_structural_relationship_edge_is_never_domain_checked() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        let catalog = catalog_with(vec![CatalogEntry {
            id: "mif-generic".to_string(),
            version: "1.0.0".to_string(),
            source: Some("core/mif-generic.yaml".to_string()),
            core: true,
        }]);
        let config = HarnessConfig { topics: vec![] };
        let concordance = json!({
            "nodes": [
                {"id": "n1", "entityType": "Concept", "topics": []},
                {"id": "n2", "entityType": "Person", "topics": []}
            ],
            "edges": [{"via": "relationship", "type": "relates-to", "source": "n1", "target": "n2"}]
        });

        let result = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap();
        assert!(result.ok(), "{:?}", result.violations);
    }

    #[test]
    fn a_relationship_edge_satisfying_declared_from_to_domains_passes() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        write(
            dir.path(),
            "engineering-base.yaml",
            "
ontology:
  id: engineering-base
  version: \"0.1.0\"
entity_types:
  - name: control
  - name: component
relationships:
  governs:
    from: [control]
    to: [component]
",
        );
        let catalog = catalog_with(vec![
            CatalogEntry {
                id: "mif-generic".to_string(),
                version: "1.0.0".to_string(),
                source: Some("core/mif-generic.yaml".to_string()),
                core: true,
            },
            CatalogEntry {
                id: "engineering-base".to_string(),
                version: "0.1.0".to_string(),
                source: Some("engineering-base.yaml".to_string()),
                core: false,
            },
        ]);
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "eng".to_string(),
                ontologies: vec!["engineering-base".to_string()],
            }],
        };
        let concordance = json!({
            "nodes": [
                {"id": "n1", "entityType": "control", "topics": ["eng"]},
                {"id": "n2", "entityType": "component", "topics": ["eng"]}
            ],
            "edges": [{"via": "relationship", "type": "governs", "source": "n1", "target": "n2"}]
        });

        let result = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap();
        assert!(result.ok(), "{:?}", result.violations);
    }

    #[test]
    fn a_subtype_satisfies_a_relationship_domain_written_against_its_supertype() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        write(
            dir.path(),
            "engineering-base.yaml",
            "
ontology:
  id: engineering-base
  version: \"0.1.0\"
entity_types:
  - name: control
  - name: component
relationships:
  governs:
    from: [control]
    to: [component]
",
        );
        write(
            dir.path(),
            "software-security.yaml",
            "
ontology:
  id: software-security
  version: \"0.1.0\"
  extends: [engineering-base]
entity_types:
  - name: security-control
    subtype_of: [control]
  - name: malware
",
        );
        let catalog = catalog_with(vec![
            CatalogEntry {
                id: "mif-generic".to_string(),
                version: "1.0.0".to_string(),
                source: Some("core/mif-generic.yaml".to_string()),
                core: true,
            },
            CatalogEntry {
                id: "engineering-base".to_string(),
                version: "0.1.0".to_string(),
                source: Some("engineering-base.yaml".to_string()),
                core: false,
            },
            CatalogEntry {
                id: "software-security".to_string(),
                version: "0.1.0".to_string(),
                source: Some("software-security.yaml".to_string()),
                core: false,
            },
        ]);
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "sec".to_string(),
                ontologies: vec!["software-security".to_string()],
            }],
        };
        let good = json!({
            "nodes": [
                {"id": "n1", "entityType": "security-control", "topics": ["sec"]},
                {"id": "n2", "entityType": "component", "topics": ["sec"]}
            ],
            "edges": [{"via": "relationship", "type": "governs", "source": "n1", "target": "n2"}]
        });
        let bad = json!({
            "nodes": [
                {"id": "n1", "entityType": "malware", "topics": ["sec"]},
                {"id": "n2", "entityType": "component", "topics": ["sec"]}
            ],
            "edges": [{"via": "relationship", "type": "governs", "source": "n1", "target": "n2"}]
        });

        let good_result = validate_concordance(&good, &config, &catalog, dir.path()).unwrap();
        assert!(good_result.ok(), "{:?}", good_result.violations);
        let bad_result = validate_concordance(&bad, &config, &catalog, dir.path()).unwrap();
        assert!(!bad_result.ok());
        assert!(bad_result.violations[0].contains("from/to domain violation"));
    }

    #[test]
    fn an_undeclared_relationship_type_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        let catalog = catalog_with(vec![CatalogEntry {
            id: "mif-generic".to_string(),
            version: "1.0.0".to_string(),
            source: Some("core/mif-generic.yaml".to_string()),
            core: true,
        }]);
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "edu".to_string(),
                ontologies: vec![],
            }],
        };
        let concordance = json!({
            "nodes": [
                {"id": "n1", "entityType": "Concept", "topics": ["edu"]},
                {"id": "n2", "entityType": "Concept", "topics": ["edu"]}
            ],
            "edges": [{"via": "relationship", "type": "made-up-relation", "source": "n1", "target": "n2"}]
        });

        let result = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap();
        assert!(!result.ok());
        assert!(
            result.violations[0]
                .contains("relationship type not MIF-core nor declared by a bound ontology")
        );
    }

    #[test]
    fn a_subtype_of_cycle_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "core/mif-generic.yaml", CORE_YAML);
        write(
            dir.path(),
            "cyclic.yaml",
            "
ontology:
  id: cyclic
  version: \"0.1.0\"
entity_types:
  - name: a
    subtype_of: [b]
  - name: b
    subtype_of: [a]
",
        );
        let catalog = catalog_with(vec![
            CatalogEntry {
                id: "mif-generic".to_string(),
                version: "1.0.0".to_string(),
                source: Some("core/mif-generic.yaml".to_string()),
                core: true,
            },
            CatalogEntry {
                id: "cyclic".to_string(),
                version: "0.1.0".to_string(),
                source: Some("cyclic.yaml".to_string()),
                core: false,
            },
        ]);
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "t".to_string(),
                ontologies: vec!["cyclic".to_string()],
            }],
        };
        let concordance = json!({"nodes": [], "edges": []});

        let error = validate_concordance(&concordance, &config, &catalog, dir.path()).unwrap_err();
        assert!(matches!(error, super::MifRhError::SubtypeOfCycle { .. }));
    }
}
