//! Drafts a new domain ontology YAML from research.
//!
//! Ports rht's `scripts/author-ontology.sh` drafting logic (Story #277):
//! either mining a topic's `ontology-map.json` for entity types actually
//! used, or scaffolding one candidate type per `mif-rh-cli
//! expansion-candidates` cluster. The `--open-pr` concierge step (branching
//! the ontologies repo, committing, opening a draft PR) stays in rht's own
//! script — it orchestrates `git`/`gh`, not `jq`, so it is out of this
//! cutover's scope.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::MifRhError;
use crate::queue::ExpansionCandidate;
use crate::resolve::MapRecord;

const TYPE_FOOTER: &str = "    base: semantic\n    traits:\n      - cited\n    schema:\n      required: []\n      properties: {}\n    source_vocab: TODO\n    source_class: TODO\n    prior_art: TODO\n    disposition: mint\n";

/// The outcome of a [`draft_from_topic`] or [`draft_from_clusters`] call.
#[derive(Debug, Clone)]
pub struct DraftReport {
    /// The rendered ontology YAML draft.
    pub yaml: String,
    /// How many candidate entity types were drafted.
    pub type_count: usize,
}

fn render_header(new_id: &str, origin: &str, from_clusters: bool) -> String {
    let mut out = String::from("---\n");
    if from_clusters {
        let _ = writeln!(
            out,
            "# {new_id} ontology — DRAFT scaffolded from recurring tier-3 misses."
        );
        let _ = writeln!(
            out,
            "# Authored by `mif-rh-cli ontology author --from-clusters` ({origin})."
        );
        out.push_str(
            "# TODO before contributing upstream: NAME each todo-cluster-N candidate for\n\
             #   the concept its excerpts share, define it, fill its grounding\n\
             #   (source_vocab / source_class / prior_art) with a cited authority, and\n\
             #   refine base/traits/schema. disposition stays 'mint' for newly-minted types.\n",
        );
    } else {
        let _ = writeln!(
            out,
            "# {new_id} ontology — DRAFT scaffolded from research topic '{origin}'."
        );
        let _ = writeln!(
            out,
            "# Authored by `mif-rh-cli ontology author` from reports/{origin}/ontology-map.json."
        );
        out.push_str(
            "# TODO before contributing upstream: fill each entity type's grounding\n\
             #   (source_vocab / source_class / prior_art) with a cited authority, and\n\
             #   refine base/traits/schema. disposition stays 'mint' for newly-minted types.\n",
        );
    }
    out.push_str("ontology:\n");
    let _ = writeln!(out, "  id: {new_id}");
    out.push_str("  version: \"0.1.0\"\n");
    let scaffolded_from = if from_clusters {
        format!("tier-3 expansion clusters ({origin})")
    } else {
        format!("research topic '{origin}'")
    };
    let _ = writeln!(
        out,
        "  description: \"DRAFT {new_id} domain ontology scaffolded from {scaffolded_from}\""
    );
    out.push_str("  extends:\n    - mif-base\n    - shared-traits\n");
    out.push_str("entity_types:\n");
    out
}

/// A JSON-quoted string literal, matching jq's `@json` filter for a plain
/// string. Serializing a `String` to JSON cannot fail; the fallback exists
/// only to satisfy this workspace's `unwrap_used`/`expect_used` lints, not
/// because the error path is reachable.
fn json_escape(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("{text:?}"))
}

/// Replaces a CR/LF/tab with a plain space, for a one-line excerpt quoted
/// in a YAML block scalar.
const fn collapse_whitespace(c: char) -> char {
    if matches!(c, '\r' | '\n' | '\t') {
        ' '
    } else {
        c
    }
}

/// Drafts a new ontology from the entity types a topic's findings used.
///
/// Mines `records` (a parsed `reports/<topic>/ontology-map.json`).
/// Generic-fallback types (resolved under `mif-generic*`) are listed
/// first — those most need a domain home of their own.
///
/// # Errors
///
/// Returns [`MifRhError::NoEntityTypesFound`] if `records` carries no typed
/// entities.
pub fn draft_from_topic(
    records: &[MapRecord],
    new_id: &str,
    topic: &str,
) -> Result<DraftReport, MifRhError> {
    let mut groups: BTreeMap<&str, bool> = BTreeMap::new();
    for record in records {
        let Some(entity_type) = record.entity_type.as_deref() else {
            continue;
        };
        let generic = record
            .resolved_ontology
            .as_deref()
            .is_some_and(|o| o.starts_with("mif-generic"));
        let entry = groups.entry(entity_type).or_insert(false);
        *entry = *entry || generic;
    }
    if groups.is_empty() {
        return Err(MifRhError::NoEntityTypesFound {
            topic: topic.to_string(),
        });
    }

    // BTreeMap iterates in alphabetical key order already; splitting into
    // generic-first then the rest (each still alphabetical) matches
    // `author-ontology.sh`'s own `group_by` + `sort_by(.generic | not)`.
    let mut ordered: Vec<&str> = groups
        .iter()
        .filter(|(_, generic)| **generic)
        .map(|(id, _)| *id)
        .collect();
    ordered.extend(
        groups
            .iter()
            .filter(|(_, generic)| !**generic)
            .map(|(id, _)| *id),
    );

    let mut yaml = render_header(new_id, topic, false);
    for entity_type in &ordered {
        let _ = writeln!(yaml, "  - name: {entity_type}");
        let _ = writeln!(
            yaml,
            "    description: \"TODO: define {entity_type} (observed in topic {topic})\""
        );
        yaml.push_str(TYPE_FOOTER);
    }
    Ok(DraftReport {
        yaml,
        type_count: ordered.len(),
    })
}

/// Drafts a new ontology from recurring tier-3 miss clusters.
///
/// Renders one candidate `todo-cluster-N` type per cluster in `clusters`
/// (`mif-rh-cli expansion-candidates` output), quoting up to three
/// representative member excerpts and every distinct member finding id.
///
/// # Errors
///
/// Returns [`MifRhError::NoClustersFound`] if `clusters` is empty.
pub fn draft_from_clusters(
    clusters: &[ExpansionCandidate],
    new_id: &str,
    clusters_source_name: &str,
) -> Result<DraftReport, MifRhError> {
    if clusters.is_empty() {
        return Err(MifRhError::NoClustersFound {
            path: clusters_source_name.to_string(),
        });
    }

    let mut yaml = render_header(new_id, clusters_source_name, true);
    for (index, cluster) in clusters.iter().enumerate() {
        let n = index + 1;
        let _ = writeln!(yaml, "  - name: todo-cluster-{n}");
        yaml.push_str("    description: |-\n");
        let _ = writeln!(
            yaml,
            "      TODO: name and define this candidate type — {} recurring tier-3 miss(es) \
             across {} run(s).",
            cluster.size, cluster.runs
        );
        yaml.push_str("      Member excerpts:\n");
        for member in cluster.members.iter().take(3) {
            let cleaned: String = member
                .content
                .chars()
                .map(collapse_whitespace)
                .take(140)
                .collect();
            let _ = writeln!(yaml, "      - {}", json_escape(&cleaned));
        }
        let mut finding_ids: Vec<&str> = cluster
            .members
            .iter()
            .map(|member| member.finding_id.as_str())
            .collect();
        finding_ids.sort_unstable();
        finding_ids.dedup();
        let _ = writeln!(yaml, "      Member findings: {}", finding_ids.join(", "));
        yaml.push_str(TYPE_FOOTER);
    }
    Ok(DraftReport {
        yaml,
        type_count: clusters.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::{draft_from_clusters, draft_from_topic};
    use crate::queue::{ClusterMember, ExpansionCandidate};
    use crate::resolve::{Basis, MapRecord};

    fn record(finding_id: &str, entity_type: Option<&str>, resolved: Option<&str>) -> MapRecord {
        MapRecord {
            finding_id: finding_id.to_string(),
            entity_type: entity_type.map(str::to_string),
            resolved_ontology: resolved.map(str::to_string),
            basis: Basis::Resolved,
            valid: true,
        }
    }

    #[test]
    fn draft_from_topic_lists_generic_fallback_types_first() {
        let records = vec![
            record("f1", Some("widget"), Some("edu-fixture@0.1.0")),
            record("f2", Some("gadget"), Some("mif-generic@1.0.0")),
            record("f3", None, None),
        ];

        let report = draft_from_topic(&records, "new-domain", "edu").unwrap();

        assert_eq!(report.type_count, 2);
        let gadget_pos = report.yaml.find("name: gadget").unwrap();
        let widget_pos = report.yaml.find("name: widget").unwrap();
        assert!(
            gadget_pos < widget_pos,
            "generic-fallback type must be listed first"
        );
        assert!(report.yaml.contains("id: new-domain"));
        assert!(report.yaml.contains("observed in topic edu"));
    }

    #[test]
    fn draft_from_topic_fails_closed_with_no_typed_entities() {
        let records = vec![record("f1", None, None)];
        let error = draft_from_topic(&records, "new-domain", "edu").unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::NoEntityTypesFound { .. }
        ));
    }

    #[test]
    fn draft_from_clusters_renders_excerpts_and_findings() {
        let clusters = vec![ExpansionCandidate {
            size: 2,
            runs: 2,
            members: vec![
                ClusterMember {
                    finding_id: "f2".to_string(),
                    topic: "edu".to_string(),
                    content: "a recurring miss".to_string(),
                    run_id: "r1".to_string(),
                },
                ClusterMember {
                    finding_id: "f1".to_string(),
                    topic: "eng".to_string(),
                    content: "another miss".to_string(),
                    run_id: "r2".to_string(),
                },
            ],
        }];

        let report = draft_from_clusters(&clusters, "new-domain", "clusters.json").unwrap();

        assert_eq!(report.type_count, 1);
        assert!(report.yaml.contains("name: todo-cluster-1"));
        assert!(report.yaml.contains("\"a recurring miss\""));
        assert!(report.yaml.contains("Member findings: f1, f2"));
    }

    #[test]
    fn draft_from_clusters_fails_closed_when_empty() {
        let error = draft_from_clusters(&[], "new-domain", "clusters.json").unwrap_err();
        assert!(matches!(error, super::MifRhError::NoClustersFound { .. }));
    }
}
