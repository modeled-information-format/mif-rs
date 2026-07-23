//! `resolve()`: classify one finding against its topic's bound ontologies.
//!
//! Deliberately 100% deterministic — exact string matching, regex
//! matching, and JSON Schema validation. No embeddings anywhere in this
//! module; see this crate's top-level documentation for why.

use std::collections::{HashMap, HashSet};

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::Catalog;
use crate::config::HarnessConfig;
use crate::error::MifRhError;
use crate::finding::Finding;
use crate::ontology_pack::OntologyPack;

/// How a finding's classification was reached, matching rht's own
/// `ontology-map.json` basis vocabulary exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Basis {
    /// Exactly one candidate ontology matched, disambiguated by an explicit
    /// `ontology.id` reference.
    Declared,
    /// Exactly one candidate ontology matched, with no ambiguity to
    /// disambiguate.
    Resolved,
    /// Classified via a discovery content pattern, not an explicit type.
    Discovery,
    /// No typing intent and no (unambiguous) discovery match.
    Untyped,
    /// Typing intent present but no ontology declares the entity type, or
    /// an explicit `ontology.id` did not resolve to a valid candidate.
    Unresolved,
    /// More than one bound ontology declares the entity type, with no
    /// `ontology.id` to disambiguate.
    Ambiguous,
}

impl Basis {
    /// The lowercase label `ontology-map.json`/`ontology-review.sh`'s own
    /// output uses for this basis (`"declared"`, `"resolved"`, ...).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Resolved => "resolved",
            Self::Discovery => "discovery",
            Self::Untyped => "untyped",
            Self::Unresolved => "unresolved",
            Self::Ambiguous => "ambiguous",
        }
    }
}

/// One `ontology-map.json` record: the result of resolving a single
/// finding, whether or not it classified cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapRecord {
    /// The finding's id.
    pub finding_id: String,
    /// The finding's entity type, if one was identified (even if
    /// unresolved/ambiguous).
    pub entity_type: Option<String>,
    /// The resolved ontology, as `"{id}@{version}"`, if classification
    /// succeeded.
    pub resolved_ontology: Option<String>,
    /// How this record's classification was reached.
    pub basis: Basis,
    /// Whether the finding's `entity` payload validated against the
    /// resolved type's schema. `false` for every basis that never reached
    /// validation (untyped/unresolved/ambiguous).
    pub valid: bool,
}

/// Everything needed to resolve one finding against one topic.
pub struct ResolveContext<'a> {
    /// The finding's topic.
    pub topic: &'a str,
    /// The enabled-ontologies catalog.
    pub catalog: &'a Catalog,
    /// The harness's topic-to-ontology bindings.
    pub config: &'a HarnessConfig,
    /// Every loaded ontology pack, keyed by id.
    pub ontology_packs: &'a HashMap<String, OntologyPack>,
}

/// The topic's *directly* bound ontology ids: every core catalog id, plus
/// the topic's own `topic_bindings` (version-checked). Deliberately **not**
/// closed over `extends` ancestors — this is the set an explicit
/// `ontology.id` must be a member of per `resolve-ontology.sh`'s own
/// contract ("an ontology.id outside the topic's bound set -> non-zero"),
/// as distinct from [`build_allowed`]'s extends-closed set of types a
/// finding may actually resolve *against*.
///
/// # Errors
///
/// Returns [`MifRhError::DirectBindingInvalid`] if a topic binding names an
/// uncataloged ontology or pins a version that does not match the catalog.
fn direct_bound_ids(ctx: &ResolveContext<'_>) -> Result<HashSet<String>, MifRhError> {
    let mut direct_ids: HashSet<String> = ctx.catalog.core_ids().map(str::to_string).collect();

    for binding in ctx.config.topic_bindings(ctx.topic) {
        let entry =
            ctx.catalog
                .find(&binding.id)
                .ok_or_else(|| MifRhError::DirectBindingInvalid {
                    topic: ctx.topic.to_string(),
                    id: binding.id.clone(),
                })?;
        if let Some(pinned) = &binding.pinned_version
            && *pinned != entry.version
        {
            return Err(MifRhError::DirectBindingInvalid {
                topic: ctx.topic.to_string(),
                id: binding.id.clone(),
            });
        }
        direct_ids.insert(binding.id);
    }

    Ok(direct_ids)
}

/// Builds the `pack.id -> OntologyMetadata` map [`mif_ontology::resolve_chain`]
/// needs, from every loaded ontology pack.
fn extends_metadata_map(
    ctx: &ResolveContext<'_>,
) -> HashMap<String, mif_ontology::OntologyMetadata> {
    ctx.ontology_packs
        .values()
        .map(|pack| {
            (
                pack.id.clone(),
                mif_ontology::OntologyMetadata {
                    id: pack.id.clone(),
                    version: pack.version.clone(),
                    description: None,
                    extends: pack.extends.clone(),
                },
            )
        })
        .collect()
}

/// Whether `target` is `oid` itself or reachable from `oid` via its
/// `extends` chain. [`mif_ontology::resolve_chain`] returns the chain in
/// base-to-specific order with `oid`'s own metadata last, so this subsumes
/// the `oid == target` case for free — no separate equality check needed.
///
/// # Errors
///
/// Returns [`MifRhError::Ontology`] if resolving `oid`'s `extends` chain
/// fails (a missing ancestor or an `extends` cycle).
fn chain_reaches(
    oid: &str,
    target: &str,
    metadata_map: &HashMap<String, mif_ontology::OntologyMetadata>,
) -> Result<bool, MifRhError> {
    let chain = mif_ontology::resolve_chain(oid, metadata_map)?;
    Ok(chain.iter().any(|resolved| resolved.id == target))
}

/// The `(allowed packs, direct bound ids, extends metadata map)` triple
/// [`build_allowed_with_context`] returns — named to keep its signature
/// under clippy's `type_complexity` threshold.
type AllowedWithContext<'a> = (
    Vec<&'a OntologyPack>,
    HashSet<String>,
    HashMap<String, mif_ontology::OntologyMetadata>,
);

/// [`build_allowed`]'s actual work, additionally returning the
/// `direct_bound_ids`/`extends_metadata_map` intermediates it computed
/// along the way, so a caller that also needs the declared-`ontology.id`
/// acceptance check (see [`resolve_finding`]) can reuse them instead of
/// recomputing both from scratch via a second `direct_bound_ids`/
/// `extends_metadata_map` call.
///
/// # Errors
///
/// Same as [`build_allowed`].
fn build_allowed_with_context<'a>(
    ctx: &ResolveContext<'a>,
) -> Result<AllowedWithContext<'a>, MifRhError> {
    let direct_ids = direct_bound_ids(ctx)?;
    let metadata_map = extends_metadata_map(ctx);

    let mut allowed_ids: HashSet<String> = HashSet::new();
    for id in &direct_ids {
        let chain = mif_ontology::resolve_chain(id, &metadata_map)?;
        allowed_ids.extend(chain.into_iter().map(|m| m.id));
    }
    allowed_ids.extend(direct_ids.iter().cloned());

    let packs = allowed_ids
        .iter()
        .filter_map(|id| ctx.ontology_packs.get(id))
        .collect();

    Ok((packs, direct_ids, metadata_map))
}

/// Resolves the set of ontologies allowed for `ctx.topic`: every core
/// ontology, every directly bound ontology (version-checked), and their
/// transitive `extends` ancestors.
///
/// # Errors
///
/// Returns [`MifRhError::DirectBindingInvalid`] if a topic binding names an
/// uncataloged ontology or pins a version that does not match the
/// catalog, or [`MifRhError::Ontology`] if resolving the `extends` chain
/// for an allowed ontology fails (a missing ancestor or an `extends`
/// cycle).
pub fn build_allowed<'a>(ctx: &ResolveContext<'a>) -> Result<Vec<&'a OntologyPack>, MifRhError> {
    build_allowed_with_context(ctx).map(|(packs, _, _)| packs)
}

fn discovery_classify(finding: &Finding, allowed: &[&OntologyPack]) -> MapRecord {
    let text = finding.discovery_text();
    let mut matches: Vec<(&str, &str)> = Vec::new();
    for pack in allowed {
        if !pack.discovery.enabled {
            continue;
        }
        for pattern in &pack.discovery.patterns {
            // file_pattern-only entries (a glob over file paths, not
            // content) are out of scope here — resolve() only ever
            // inspects a finding's content.
            let Some(content_pattern) = &pattern.content_pattern else {
                continue;
            };
            let Ok(re) = RegexBuilder::new(content_pattern)
                .case_insensitive(true)
                .build()
            else {
                continue;
            };
            if re.is_match(&text)
                && let Some(entity_type) = pattern.suggest_entity.as_deref()
            {
                matches.push((entity_type, pack.id.as_str()));
            }
        }
    }

    let unique: HashSet<(&str, &str)> = matches.iter().copied().collect();
    if unique.len() == 1 {
        let (entity_type, ontology_id) = matches[0];
        let version = allowed
            .iter()
            .find(|p| p.id == ontology_id)
            .map_or_else(String::new, |p| p.version.clone());
        MapRecord {
            finding_id: finding.id.clone(),
            entity_type: Some(entity_type.to_string()),
            resolved_ontology: Some(format!("{ontology_id}@{version}")),
            basis: Basis::Discovery,
            valid: true,
        }
    } else {
        // 0 matches, or >1 conflicting discovery matches: both fall back to
        // untyped rather than guessing — matches rht's own discovery
        // classifier, which never picks among ambiguous discovery hits.
        MapRecord {
            finding_id: finding.id.clone(),
            entity_type: None,
            resolved_ontology: None,
            basis: Basis::Untyped,
            valid: true,
        }
    }
}

fn unresolved(finding_id: &str, entity_type: Option<&str>) -> MapRecord {
    MapRecord {
        finding_id: finding_id.to_string(),
        entity_type: entity_type.map(str::to_string),
        resolved_ontology: None,
        basis: Basis::Unresolved,
        valid: false,
    }
}

/// Builds a full, additive JSON Schema from an entity type's `{required,
/// properties}` shape (rht's ontology packs never set `additionalProperties:
/// false`).
fn full_entity_schema(entity_type_schema: &Value) -> Value {
    let required = entity_type_schema
        .get("required")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let properties = entity_type_schema
        .get("properties")
        .cloned()
        .unwrap_or_else(|| json!({}));
    json!({
        "type": "object",
        "additionalProperties": true,
        "required": required,
        "properties": properties,
    })
}

fn validate_entity(
    entity: Option<&Value>,
    entity_type: &str,
    schema: &Value,
) -> Result<bool, MifRhError> {
    let Some(entity) = entity else {
        return Ok(false);
    };
    let full_schema = full_entity_schema(schema);
    let validator = jsonschema::options()
        .build(&full_schema)
        .map_err(|source| MifRhError::EntityTypeSchemaInvalid {
            entity_type: entity_type.to_string(),
            detail: source.to_string(),
        })?;
    Ok(validator.is_valid(entity))
}

/// Resolves one finding, producing an `ontology-map.json` record.
///
/// This is `resolve-ontology.sh`'s exact algorithm: discovery fallback when
/// no typing intent is present, otherwise exact `entity_type` matching
/// against the topic's allowed ontologies (core, direct bindings, and their
/// `extends` ancestors), disambiguated by an explicit `ontology.id` when
/// more than one candidate declares the type, followed by additive JSON
/// Schema validation of the `entity` payload. A finding that fails to
/// classify or fails validation is still recorded (`valid: false`), not
/// treated as an error — see this module's [`Basis`] documentation.
///
/// # Errors
///
/// Returns [`MifRhError::DirectBindingInvalid`] or [`MifRhError::Ontology`]
/// if the topic's own ontology bindings cannot be resolved at all (see
/// [`build_allowed`]) — a setup problem, not a per-finding classification
/// outcome. Returns [`MifRhError::EntityTypeSchemaInvalid`] if the resolved
/// entity type's own schema is malformed.
pub fn resolve_finding(
    finding: &Finding,
    ctx: &ResolveContext<'_>,
) -> Result<MapRecord, MifRhError> {
    if !finding.has_typing_intent() {
        let allowed = build_allowed(ctx)?;
        return Ok(discovery_classify(finding, &allowed));
    }

    let Some(entity_type) = finding.entity_type() else {
        return Ok(unresolved(&finding.id, None));
    };

    let (allowed, direct_ids, metadata_map) = build_allowed_with_context(ctx)?;
    let matches: Vec<(&OntologyPack, &crate::ontology_pack::EntityType)> = allowed
        .iter()
        .filter_map(|pack| {
            pack.entity_types
                .iter()
                .find(|et| et.name == entity_type)
                .map(|def| (*pack, def))
        })
        .collect();

    let declared_ontology_id = finding.ontology.as_ref().map(|o| o.id.as_str());

    let (pack, entity_type_def, basis) = match matches.len() {
        0 => return Ok(unresolved(&finding.id, Some(entity_type))),
        1 => {
            let (pack, def) = matches[0];
            match declared_ontology_id {
                Some(oid) => {
                    // Accept only when `oid` is one of the topic's directly
                    // bound ontologies (never a same-id equality check
                    // alone — an un-bound base layer named directly, e.g.
                    // `ontology.id: engineering-base`, must NOT resolve
                    // just because it happens to equal the declaring
                    // pack's id) AND `oid`'s own `extends` chain actually
                    // reaches the pack that declares this entity type. The
                    // `oid == pack.id` case is subsumed by `chain_reaches`
                    // for free (see its doc comment) — no separate
                    // fast-path branch needed. `direct_ids`/`metadata_map`
                    // are already the ones `build_allowed_with_context`
                    // computed above; no need to recompute them here.
                    let accepted =
                        direct_ids.contains(oid) && chain_reaches(oid, &pack.id, &metadata_map)?;
                    if accepted {
                        (pack, def, Basis::Declared)
                    } else {
                        return Ok(unresolved(&finding.id, Some(entity_type)));
                    }
                },
                None => (pack, def, Basis::Resolved),
            }
        },
        _ => {
            let Some(oid) = declared_ontology_id else {
                return Ok(MapRecord {
                    finding_id: finding.id.clone(),
                    entity_type: Some(entity_type.to_string()),
                    resolved_ontology: None,
                    basis: Basis::Ambiguous,
                    valid: false,
                });
            };
            match matches.into_iter().find(|(pack, _)| pack.id == oid) {
                Some((pack, def)) => (pack, def, Basis::Declared),
                // An `ontology.id` that names none of the ambiguous candidates
                // is still an ambiguous classification, not a fresh
                // "unresolved" one — matches resolve-ontology.sh's own
                // `mcount > 1` branch, which records "ambiguous" whether the
                // declared id is absent or simply doesn't match.
                None => {
                    return Ok(MapRecord {
                        finding_id: finding.id.clone(),
                        entity_type: Some(entity_type.to_string()),
                        resolved_ontology: None,
                        basis: Basis::Ambiguous,
                        valid: false,
                    });
                },
            }
        },
    };

    let valid = validate_entity(
        finding.entity.as_ref(),
        entity_type,
        &entity_type_def.schema,
    )?;

    Ok(MapRecord {
        finding_id: finding.id.clone(),
        entity_type: Some(entity_type.to_string()),
        resolved_ontology: Some(format!("{}@{}", pack.id, pack.version)),
        basis,
        valid,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{Basis, ResolveContext, resolve_finding};
    use crate::catalog::{Catalog, CatalogEntry};
    use crate::config::{HarnessConfig, TopicConfig};
    use crate::finding::Finding;
    use crate::ontology_pack::parse_pack;

    fn finding_from_json(value: serde_json::Value) -> Finding {
        serde_json::from_value(value).unwrap()
    }

    fn edu_fixture_pack() -> crate::ontology_pack::OntologyPack {
        parse_pack(
            "
ontology:
  id: edu-fixture
  version: \"0.1.0\"
entity_types:
  - name: title
    schema:
      required: [name, isbn]
      properties: {name: {type: string}, isbn: {type: string}}
discovery:
  enabled: true
  patterns:
    - content_pattern: \"\\\\b(ISBN|textbook)\\\\b\"
      suggest_entity: title
",
            "edu-fixture.yaml",
        )
        .unwrap()
    }

    fn ctx_fixture<'a>(
        packs: &'a HashMap<String, crate::ontology_pack::OntologyPack>,
        catalog: &'a Catalog,
        config: &'a HarnessConfig,
        topic: &'a str,
    ) -> ResolveContext<'a> {
        ResolveContext {
            topic,
            catalog,
            config,
            ontology_packs: packs,
        }
    }

    fn edu_setup() -> (
        HashMap<String, crate::ontology_pack::OntologyPack>,
        Catalog,
        HarnessConfig,
    ) {
        let mut packs = HashMap::new();
        packs.insert("edu-fixture".to_string(), edu_fixture_pack());
        let catalog = Catalog {
            ontologies: vec![CatalogEntry {
                id: "edu-fixture".to_string(),
                version: "0.1.0".to_string(),
                source: None,
                core: false,
            }],
        };
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "edu".to_string(),
                ontologies: vec!["edu-fixture".to_string()],
            }],
        };
        (packs, catalog, config)
    }

    #[test]
    fn resolves_a_declared_type_and_validates_the_entity() {
        let (packs, catalog, config) = edu_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "edu");
        let finding = finding_from_json(json!({
            "@id": "f-good",
            "entity": {"name": "Algebra I", "entity_type": "title", "isbn": "9780000000002"}
        }));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Resolved);
        assert!(record.valid);
        assert_eq!(
            record.resolved_ontology.as_deref(),
            Some("edu-fixture@0.1.0")
        );
    }

    #[test]
    fn extra_properties_are_allowed_additively() {
        let (packs, catalog, config) = edu_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "edu");
        let finding = finding_from_json(json!({
            "@id": "f-extra",
            "entity": {"name": "Algebra I", "entity_type": "title", "isbn": "9780000000002", "vibe": "x"}
        }));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert!(record.valid);
    }

    #[test]
    fn missing_required_field_fails_validation_but_still_records() {
        let (packs, catalog, config) = edu_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "edu");
        let finding = finding_from_json(json!({
            "@id": "f-missing",
            "entity": {"entity_type": "title"}
        }));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Resolved);
        assert!(!record.valid);
    }

    #[test]
    fn undeclared_type_is_unresolved() {
        let (packs, catalog, config) = edu_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "edu");
        let finding = finding_from_json(json!({
            "@id": "f-undecl",
            "entity": {"entity_type": "not-a-type"}
        }));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Unresolved);
        assert!(!record.valid);
    }

    #[test]
    fn untyped_finding_with_no_discovery_match_is_untyped() {
        let (packs, catalog, config) = edu_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "edu");
        let finding = finding_from_json(json!({"@id": "f-untyped", "content": "nothing special"}));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Untyped);
        assert!(record.valid);
        assert_eq!(record.entity_type, None);
    }

    #[test]
    fn discovery_pattern_classifies_an_untyped_finding() {
        let (packs, catalog, config) = edu_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "edu");
        let finding = finding_from_json(
            json!({"@id": "f-discovery", "content": "a great textbook, ISBN included"}),
        );

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Discovery);
        assert_eq!(record.entity_type.as_deref(), Some("title"));
        assert!(record.valid);
    }

    #[test]
    fn declared_type_on_an_unbound_topic_only_resolves_core() {
        let (packs, catalog, config) = edu_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "bare");
        let finding = finding_from_json(json!({
            "@id": "f-good",
            "entity": {"name": "x", "entity_type": "title", "isbn": "y"}
        }));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Unresolved);
    }

    #[test]
    fn ambiguous_type_across_two_bound_ontologies_without_explicit_id() {
        let mut packs = HashMap::new();
        packs.insert(
            "a".to_string(),
            parse_pack(
                "ontology:\n  id: a\n  version: \"1.0.0\"\nentity_types:\n  - name: technology\n    schema: {}\n",
                "a.yaml",
            )
            .unwrap(),
        );
        packs.insert(
            "b".to_string(),
            parse_pack(
                "ontology:\n  id: b\n  version: \"1.0.0\"\nentity_types:\n  - name: technology\n    schema: {}\n",
                "b.yaml",
            )
            .unwrap(),
        );
        let catalog = Catalog {
            ontologies: vec![
                CatalogEntry {
                    id: "a".to_string(),
                    version: "1.0.0".to_string(),
                    source: None,
                    core: false,
                },
                CatalogEntry {
                    id: "b".to_string(),
                    version: "1.0.0".to_string(),
                    source: None,
                    core: false,
                },
            ],
        };
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "eng".to_string(),
                ontologies: vec!["a".to_string(), "b".to_string()],
            }],
        };
        let ctx = ctx_fixture(&packs, &catalog, &config, "eng");

        let ambiguous = finding_from_json(json!({
            "@id": "f-amb",
            "entity": {"entity_type": "technology"}
        }));
        let record = resolve_finding(&ambiguous, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Ambiguous);

        let disambiguated = finding_from_json(json!({
            "@id": "f-disambig",
            "entity": {"entity_type": "technology"},
            "ontology": {"id": "b"}
        }));
        let record = resolve_finding(&disambiguated, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Declared);
        assert_eq!(record.resolved_ontology.as_deref(), Some("b@1.0.0"));

        // An explicit `ontology.id` that names neither candidate is STILL
        // ambiguous, not unresolved — matches resolve-ontology.sh's
        // `mcount > 1` branch (`grep -qx "$oid" || record ... "ambiguous"`),
        // which never falls back to "unresolved" once there are 2+ matches.
        let wrong_id = finding_from_json(json!({
            "@id": "f-wrong-id",
            "entity": {"entity_type": "technology"},
            "ontology": {"id": "c"}
        }));
        let record = resolve_finding(&wrong_id, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Ambiguous);
        assert!(!record.valid);
    }

    #[test]
    fn file_pattern_only_discovery_entry_is_skipped_not_treated_as_match_all() {
        // Matches rht's own `software-engineering.ontology.yaml`, which mixes
        // file_pattern-only entries (no `content_pattern`) with real content
        // patterns. A file_pattern-only entry must never be treated as an
        // empty-string content regex (which would match every finding).
        let mut packs = HashMap::new();
        packs.insert(
            "se-fixture".to_string(),
            parse_pack(
                "
ontology:
  id: se-fixture
  version: \"0.1.0\"
entity_types:
  - name: decision
    schema: {}
discovery:
  enabled: true
  patterns:
    - file_pattern: \"*.md\"
    - content_pattern: \"\\\\bADR\\\\b\"
      suggest_entity: decision
",
                "se-fixture.yaml",
            )
            .unwrap(),
        );
        let catalog = Catalog {
            ontologies: vec![CatalogEntry {
                id: "se-fixture".to_string(),
                version: "0.1.0".to_string(),
                source: None,
                core: false,
            }],
        };
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "eng".to_string(),
                ontologies: vec!["se-fixture".to_string()],
            }],
        };
        let ctx = ctx_fixture(&packs, &catalog, &config, "eng");

        // Content that shares no words with the real content_pattern, and
        // would only match if the file_pattern-only entry were incorrectly
        // treated as an always-matching content regex.
        let finding = finding_from_json(json!({
            "@id": "f-no-match",
            "content": "totally unrelated prose about lunch"
        }));
        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Untyped);
        assert_eq!(record.entity_type, None);
    }

    #[test]
    fn topic_binding_an_uncataloged_ontology_is_a_hard_error() {
        let (packs, catalog, _config) = edu_setup();
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "edu".to_string(),
                ontologies: vec!["not-cataloged".to_string()],
            }],
        };
        let ctx = ctx_fixture(&packs, &catalog, &config, "edu");
        let finding = finding_from_json(json!({"@id": "f-x", "content": "x"}));

        let error = resolve_finding(&finding, &ctx).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::DirectBindingInvalid { .. }
        ));
    }

    /// Two-pack `extends` fixture matching issue #135's repro: `base`
    /// declares an entity type and has no `extends` of its own; `descendant`
    /// extends `[base]` and declares no entity types of its own, relying on
    /// the chain. Only `descendant` is bound to the `"eng"` topic — `base`
    /// is a shared layer reached solely via the chain, exactly like
    /// `engineering-base`/`software-engineering` in the real corpus.
    fn extends_chain_setup() -> (
        HashMap<String, crate::ontology_pack::OntologyPack>,
        Catalog,
        HarnessConfig,
    ) {
        let mut packs = HashMap::new();
        packs.insert(
            "base".to_string(),
            parse_pack(
                "ontology:\n  id: base\n  version: \"0.1.0\"\nentity_types:\n  - name: decision\n    schema: {}\n",
                "base.yaml",
            )
            .unwrap(),
        );
        packs.insert(
            "descendant".to_string(),
            parse_pack(
                "ontology:\n  id: descendant\n  version: \"0.1.0\"\n  extends: [base]\nentity_types: []\n",
                "descendant.yaml",
            )
            .unwrap(),
        );
        let catalog = Catalog {
            ontologies: vec![
                CatalogEntry {
                    id: "base".to_string(),
                    version: "0.1.0".to_string(),
                    source: None,
                    core: false,
                },
                CatalogEntry {
                    id: "descendant".to_string(),
                    version: "0.1.0".to_string(),
                    source: None,
                    core: false,
                },
            ],
        };
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "eng".to_string(),
                // Only the descendant is bound directly — `base` is reached
                // solely through `descendant`'s `extends` chain, never
                // bound to the topic itself.
                ontologies: vec!["descendant".to_string()],
            }],
        };
        (packs, catalog, config)
    }

    #[test]
    fn extends_chain_type_resolves_via_topic_bound_descendant_id() {
        let (packs, catalog, config) = extends_chain_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "eng");
        let finding = finding_from_json(json!({
            "@id": "f-extends-declared",
            "entity": {"entity_type": "decision"},
            "ontology": {"id": "descendant"}
        }));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Declared);
        assert!(record.valid);
        // Load-bearing: `resolved_ontology` names the pack that actually
        // DECLARES the schema (`base`), never the pinned `ontology.id`
        // (`descendant`) — matches the existing parity test
        // `transitive_extends_does_not_leak_across_unrelated_topics`
        // (crates/mif-rh/tests/parity.rs), which asserts the identical
        // convention for an implicit/None `ontology.id`, and
        // `find_pin_safety_gaps` (crates/mif-rh/src/vendor.rs), which keys
        // pin-safety-gap detection off `resolved_ontology` naming the
        // schema-declaring pack.
        assert_eq!(record.resolved_ontology.as_deref(), Some("base@0.1.0"));
    }

    #[test]
    fn extends_chain_base_layer_named_directly_is_rejected() {
        let (packs, catalog, config) = extends_chain_setup();
        let ctx = ctx_fixture(&packs, &catalog, &config, "eng");
        // `base` is NOT one of the topic's directly bound ontologies (only
        // `descendant` is) — naming it directly must be rejected per
        // resolve-ontology.sh's own contract ("an ontology.id outside the
        // topic's bound set -> non-zero"), even though `base` is the exact
        // pack that declares this entity type.
        let finding = finding_from_json(json!({
            "@id": "f-extends-base-direct",
            "entity": {"entity_type": "decision"},
            "ontology": {"id": "base"}
        }));

        let record = resolve_finding(&finding, &ctx).unwrap();
        assert_eq!(record.basis, Basis::Unresolved);
        assert!(!record.valid);
    }
}
