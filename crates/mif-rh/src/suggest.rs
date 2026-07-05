//! Embedding-based entity-type suggestion with confidence tiers.
//!
//! The suggestion core behind `mif-rh-mcp`'s `suggest_type` tool and
//! `mif-rh-cli suggest-type`: embeds a query text and every allowed entity
//! type's positive embedding document
//! ([`mif_ontology::EntityType::embedding_doc`] — description + aliases +
//! exemplars), ranks by cosine similarity, and annotates each candidate
//! with its [`ConfidenceTier`] under the corpus's calibration artifact
//! (MIF ADR-020's two-threshold, three-tier policy).
//!
//! Types carrying curated `negative_examples` additionally participate in
//! the negative-demotion-v1 gate
//! ([`mif_ontology::confidence::negative_demotes`]): each negative example
//! is embedded separately, the query's best similarity against them is the
//! candidate's negative evidence, and a candidate whose negative evidence
//! reaches its positive score is barred from tier 1 — raw scores and
//! ranking stay untouched. The gate is data-driven: a corpus whose packs
//! carry no negatives embeds nothing extra and scores byte-identically to
//! the pre-gate engine.
//!
//! Suggestions are hypotheses, never stamps: nothing here writes to
//! `reports/`, and even a [`ConfidenceTier::AutoClassifyEligible`]
//! candidate requires a confirming agent/human action to become a
//! finding's `entity_type`. This module lives in the hypothesis layer of
//! this crate's determinism boundary (see the crate doc) — `resolve`/
//! `review` never call it.

use mif_ontology::{CalibrationConfig, ConfidenceTier, assign_tier_with_negatives};
use serde::{Deserialize, Serialize};

use crate::error::MifRhError;
use crate::index::cosine_similarity;
use crate::resolve::{ResolveContext, build_allowed};

/// The shared candidate-list depth.
///
/// `review --suggest` queues this many ranked candidates per finding, and
/// `calibrate` measures gold recall at exactly this depth — the two are
/// semantically coupled (tier-2 recall is calibrated at the queue's own
/// truncation depth), so they share one constant.
pub const SUGGESTION_DEPTH: usize = 5;

/// One ranked, tier-annotated entity-type hypothesis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeSuggestion {
    /// The candidate entity type's name.
    pub entity_type: String,
    /// The ontology declaring it.
    pub ontology_id: String,
    /// Raw cosine similarity between the query and the type's positive
    /// embedding document (higher is more similar).
    pub score: f32,
    /// The confidence tier this score falls into under the calibration in
    /// force.
    pub tier: ConfidenceTier,
    /// The top candidate's lead over the second-best candidate. `Some`
    /// only at rank 0 when a rival exists — the margin is a property of
    /// the decision between the top two, not of every candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<f32>,
    /// Whether the tier came from a real calibration run against the
    /// embedding model actually in use. `false` means built-in
    /// uncalibrated defaults (or a calibration for a different model) —
    /// the tier is advisory shape, not a governed threshold.
    pub calibrated: bool,
    /// Whether the negative-demotion-v1 gate barred this candidate from
    /// tier 1: the query sat at least as close to one of the type's
    /// curated `negative_examples` as to its positive document. Absent
    /// from the wire when `false` (including for every type carrying no
    /// negatives).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub negative_demoted: bool,
}

/// A topic's embedded candidate set: every allowed entity type with a
/// positive embedding signal, its document pre-embedded once.
///
/// Callers scoring many queries against one topic (`calibrate`,
/// `review --suggest`) build this once and reuse it — embedding the
/// candidate documents per query would cost `O(queries x types)` forward
/// passes where `O(queries + types)` suffices.
#[derive(Debug)]
pub struct CandidateSet {
    /// One embedded candidate per allowed entity type with positive signal.
    candidates: Vec<Candidate>,
    /// Whether the calibration in force governs the embedding model in use.
    calibrated: bool,
}

/// One pre-embedded candidate entity type.
#[derive(Debug)]
struct Candidate {
    /// The entity type's name.
    name: String,
    /// The ontology declaring it.
    ontology_id: String,
    /// The embedded positive document (description + aliases + exemplars).
    vector: Vec<f32>,
    /// One embedding per curated negative example; empty for the many
    /// types that carry none, which therefore cost nothing extra to score.
    negative_vectors: Vec<Vec<f32>>,
}

/// Embeds `ctx.topic`'s allowed entity-type documents once, for reuse
/// across many queries.
///
/// # Errors
///
/// Returns [`MifRhError`] if the topic's ontology bindings cannot be
/// resolved or a candidate document cannot be embedded.
pub fn build_candidates(
    ctx: &ResolveContext<'_>,
    embedder: &mif_embed::Embedder,
    cal: &CalibrationConfig,
) -> Result<CandidateSet, MifRhError> {
    let allowed = build_allowed(ctx)?;
    let mut candidates = Vec::new();
    for pack in &allowed {
        for entity_type in &pack.entity_types {
            let Some(doc) = entity_type.embedding_doc() else {
                continue;
            };
            candidates.push(Candidate {
                name: entity_type.name.clone(),
                ontology_id: pack.id.clone(),
                vector: embedder.embed(&doc)?,
                negative_vectors: embed_negatives(entity_type, embedder)?,
            });
        }
    }
    Ok(CandidateSet {
        candidates,
        // The calibration only governs scores produced by the model
        // actually in use; an artifact naming any other model reads as
        // uncalibrated.
        calibrated: cal.governs(mif_embed::MODEL_ID),
    })
}

/// Embeds one type's curated negative examples, each separately: the gate
/// compares the query against the closest single near-miss, and
/// concatenating negatives would blur them into a document that resembles
/// none of its members. Empty for the many types carrying no negatives.
fn embed_negatives(
    entity_type: &mif_ontology::EntityType,
    embedder: &mif_embed::Embedder,
) -> Result<Vec<Vec<f32>>, MifRhError> {
    let mut vectors = Vec::new();
    for negative in &entity_type.negative_examples {
        if negative.trim().is_empty() {
            continue;
        }
        vectors.push(embedder.embed(negative)?);
    }
    Ok(vectors)
}

/// Ranks a pre-embedded query against a topic's [`CandidateSet`] and
/// annotates confidence tiers. Pure scoring — no embedding happens here.
#[must_use]
pub fn suggest_from_candidates(
    query_vector: &[f32],
    set: &CandidateSet,
    cal: &CalibrationConfig,
    limit: usize,
) -> Vec<TypeSuggestion> {
    let mut scored: Vec<(&str, &str, f32, &[Vec<f32>])> = set
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.name.as_str(),
                candidate.ontology_id.as_str(),
                cosine_similarity(query_vector, &candidate.vector),
                candidate.negative_vectors.as_slice(),
            )
        })
        .collect();

    // Total order: score desc, then ontology id, then type name — exact
    // score ties (identical embedding docs across packs) must rank
    // deterministically, including which twin sits at rank 0 carrying the
    // margin, since build_allowed's pack order is hash-map-dependent.
    // Negative evidence never reorders: demotion changes a candidate's
    // tier, not its rank or score.
    scored.sort_by(|a, b| {
        b.2.total_cmp(&a.2)
            .then_with(|| a.1.cmp(b.1))
            .then_with(|| a.0.cmp(b.0))
    });
    let second_best = scored.get(1).map(|(_, _, score, _)| *score);
    // Negative evidence is computed only for the candidates actually
    // returned: a truncated-away candidate's tier is discarded, and a
    // demoted non-top candidate lands in the same band assign_tier would
    // give it anyway, so scoring negatives for the full set would be pure
    // waste on large ontologies. The margin above was already captured
    // against the true second-best of the full ranking.
    scored.truncate(limit);

    scored
        .into_iter()
        .enumerate()
        .map(
            |(rank, (entity_type, ontology_id, score, negative_vectors))| {
                // The candidate's negative evidence: the query's best
                // similarity to any single curated near-miss (None when the
                // type carries none — the gate then never engages).
                let negative_similarity = negative_vectors
                    .iter()
                    .map(|negative| cosine_similarity(query_vector, negative))
                    .reduce(f32::max);
                TypeSuggestion {
                    entity_type: entity_type.to_string(),
                    ontology_id: ontology_id.to_string(),
                    score,
                    tier: assign_tier_with_negatives(
                        rank,
                        score,
                        second_best,
                        negative_similarity,
                        cal,
                    ),
                    margin: (rank == 0)
                        .then(|| second_best.map(|second| score - second))
                        .flatten(),
                    calibrated: set.calibrated,
                    negative_demoted: mif_ontology::negative_demotes(score, negative_similarity),
                }
            },
        )
        .collect()
}

/// Suggests candidate entity types for `text` against `ctx.topic`'s
/// allowed ontologies, ranked by similarity and annotated with confidence
/// tiers.
///
/// Entity types with no positive embedding signal (no description, aliases,
/// or exemplars) are skipped. The ranking and the top candidate's margin
/// are computed over the full candidate set *before* truncation to
/// `limit`, so the margin is always measured against the true second-best,
/// not the best survivor of an early cut; per-candidate negative evidence
/// and tiers are then evaluated for the returned candidates only.
///
/// The second-best candidate includes duplicate declarations of the same
/// type name across allowed packs: two packs declaring near-identical
/// types leave the near-twin at rank 1 with a margin near zero, so the top
/// candidate cannot reach tier 1 even when the type-*name* answer is
/// unambiguous. Deliberate — the `ontology_id` genuinely is ambiguous in
/// that corpus, and auto-classify eligibility must not paper over it.
///
/// # Errors
///
/// Returns [`MifRhError`] if the topic's ontology bindings cannot be
/// resolved, or the embedding model cannot embed the query or a candidate
/// document.
pub fn suggest_type(
    text: &str,
    ctx: &ResolveContext<'_>,
    embedder: &mif_embed::Embedder,
    cal: &CalibrationConfig,
    limit: usize,
) -> Result<Vec<TypeSuggestion>, MifRhError> {
    let set = build_candidates(ctx, embedder, cal)?;
    let query_vector = embedder.embed(text)?;
    Ok(suggest_from_candidates(&query_vector, &set, cal, limit))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mif_ontology::{CalibrationConfig, ConfidenceTier};

    use super::{Candidate, CandidateSet, suggest_from_candidates, suggest_type};
    use crate::catalog::{Catalog, CatalogEntry};
    use crate::config::HarnessConfig;
    use crate::ontology_pack::parse_pack;
    use crate::resolve::ResolveContext;

    const ENRICHED_PACK_YAML: &str = "
ontology:
  id: sec-fixture
  version: \"0.1.0\"
entity_types:
  - name: control
    description: A security safeguard applied to a system
    aliases: [countermeasure]
    exemplars:
      - Enforce MFA for all administrative access
  - name: incident
    description: A security event that caused harm
  - name: undescribed
";

    fn fixture() -> (Catalog, HarnessConfig, HashMap<String, crate::OntologyPack>) {
        let pack = parse_pack(ENRICHED_PACK_YAML, "sec-fixture.yaml").unwrap();
        let catalog = Catalog {
            ontologies: vec![CatalogEntry {
                id: "sec-fixture".to_string(),
                version: "0.1.0".to_string(),
                source: None,
                core: true,
            }],
        };
        let config: HarnessConfig = serde_json::from_value(serde_json::json!({
            "topics": [{ "id": "sec", "ontologies": [] }]
        }))
        .unwrap();
        let mut packs = HashMap::new();
        packs.insert(pack.id.clone(), pack);
        (catalog, config, packs)
    }

    /// A calibrated config whose floors the hand-built vectors below can
    /// clear: tier1 floor 0.85, margin 0.05, tier2 floor 0.60.
    fn calibrated() -> CalibrationConfig {
        CalibrationConfig {
            calibrated: true,
            ..CalibrationConfig::default()
        }
    }

    fn candidate(name: &str, vector: Vec<f32>, negative_vectors: Vec<Vec<f32>>) -> Candidate {
        Candidate {
            name: name.to_string(),
            ontology_id: "pack".to_string(),
            vector,
            negative_vectors,
        }
    }

    #[test]
    fn negative_evidence_demotes_the_top_candidate_without_reordering() {
        // The query aligns perfectly with `a`'s positive doc (cos 1.0) —
        // floor and margin both clear — but equally with one of `a`'s
        // curated negatives, so the gate bars tier 1. Rank and score are
        // untouched: `a` still leads.
        let query = [1.0, 0.0];
        let set = CandidateSet {
            candidates: vec![
                candidate("a", vec![1.0, 0.0], vec![vec![0.0, 1.0], vec![1.0, 0.0]]),
                candidate("b", vec![0.6, 0.8], vec![]),
            ],
            calibrated: true,
        };

        let suggestions = suggest_from_candidates(&query, &set, &calibrated(), 10);

        assert_eq!(suggestions[0].entity_type, "a");
        assert!(suggestions[0].negative_demoted);
        assert_eq!(suggestions[0].tier, ConfidenceTier::FlagForReview);
        // The demoted candidate keeps its rank-0 margin over `b`.
        assert!(suggestions[0].margin.is_some());
        assert!(!suggestions[1].negative_demoted);
    }

    #[test]
    fn weak_negative_evidence_leaves_tier_one_reachable() {
        // `a` carries a negative, but the query is nearly orthogonal to it
        // (cos 0.0) and aligned with the positive doc: no demotion.
        let query = [1.0, 0.0];
        let set = CandidateSet {
            candidates: vec![
                candidate("a", vec![1.0, 0.0], vec![vec![0.0, 1.0]]),
                candidate("b", vec![0.6, 0.8], vec![]),
            ],
            calibrated: true,
        };

        let suggestions = suggest_from_candidates(&query, &set, &calibrated(), 10);

        assert!(!suggestions[0].negative_demoted);
        assert_eq!(suggestions[0].tier, ConfidenceTier::AutoClassifyEligible);
    }

    #[test]
    fn candidates_without_negatives_score_exactly_as_before_the_gate() {
        // The same set with and without an irrelevant negative on the OTHER
        // candidate: the no-negatives candidate's whole suggestion record
        // is identical, proving the unpenalized path unchanged.
        let query = [1.0, 0.0];
        let bare = CandidateSet {
            candidates: vec![
                candidate("a", vec![1.0, 0.0], vec![]),
                candidate("b", vec![0.6, 0.8], vec![]),
            ],
            calibrated: true,
        };
        let with_unrelated_negative = CandidateSet {
            candidates: vec![
                candidate("a", vec![1.0, 0.0], vec![]),
                candidate("b", vec![0.6, 0.8], vec![vec![0.0, 1.0]]),
            ],
            calibrated: true,
        };

        let before = suggest_from_candidates(&query, &bare, &calibrated(), 10);
        let after = suggest_from_candidates(&query, &with_unrelated_negative, &calibrated(), 10);

        assert_eq!(before, after);
        assert_eq!(before[0].tier, ConfidenceTier::AutoClassifyEligible);
    }

    #[test]
    fn the_default_calibration_model_names_the_embedder_actually_in_use() {
        // Guards the governs() rule against silent drift: if mif-embed ever
        // changes models, this forces the calibration default to move with
        // it (or the divergence to be handled deliberately).
        assert_eq!(
            mif_ontology::confidence::DEFAULT_EMBEDDING_MODEL,
            mif_embed::MODEL_ID
        );
    }

    #[test]
    fn suggests_tier_annotated_candidates_skipping_signal_less_types() {
        let Ok(embedder) = mif_embed::Embedder::load() else {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        };
        let (catalog, config, packs) = fixture();
        let ctx = ResolveContext {
            topic: "sec",
            catalog: &catalog,
            config: &config,
            ontology_packs: &packs,
        };
        let cal = CalibrationConfig::default();

        let suggestions = suggest_type(
            "Require multi-factor authentication as a countermeasure on admin logins",
            &ctx,
            &embedder,
            &cal,
            10,
        )
        .unwrap();

        // `undescribed` has no positive signal and must be skipped.
        assert_eq!(suggestions.len(), 2);
        assert!(suggestions.iter().all(|s| s.entity_type != "undescribed"));
        // The alias/exemplar-enriched `control` type must outrank `incident`
        // for an MFA/countermeasure query.
        assert_eq!(suggestions[0].entity_type, "control");
        assert!(suggestions[0].score >= suggestions[1].score);
        // Rank 0 carries the margin; rank 1 does not.
        assert!(suggestions[0].margin.is_some());
        assert!(suggestions[1].margin.is_none());
        // Built-in defaults are explicitly uncalibrated.
        assert!(!suggestions[0].calibrated);
    }

    const NEGATIVES_PACK_YAML: &str = "
ontology:
  id: sec-fixture
  version: \"0.1.0\"
entity_types:
  - name: control
    description: A security safeguard applied to a system
    aliases: [countermeasure]
  - name: incident
    description: A security event that caused harm
    negative_examples:
      - Require multi-factor authentication on admin logins
";

    #[test]
    fn a_curated_negative_demotes_its_type_end_to_end() {
        let Ok(embedder) = mif_embed::Embedder::load() else {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        };
        let pack = parse_pack(NEGATIVES_PACK_YAML, "sec-fixture.yaml").unwrap();
        let catalog = Catalog {
            ontologies: vec![CatalogEntry {
                id: "sec-fixture".to_string(),
                version: "0.1.0".to_string(),
                source: None,
                core: true,
            }],
        };
        let config: HarnessConfig = serde_json::from_value(serde_json::json!({
            "topics": [{ "id": "sec", "ontologies": [] }]
        }))
        .unwrap();
        let mut packs = HashMap::new();
        packs.insert(pack.id.clone(), pack);
        let ctx = ResolveContext {
            topic: "sec",
            catalog: &catalog,
            config: &config,
            ontology_packs: &packs,
        };

        // The query IS incident's curated negative: its similarity to that
        // negative (~1.0) must reach any positive score incident earns, so
        // incident is demoted; control carries no negatives and is not.
        let suggestions = suggest_type(
            "Require multi-factor authentication on admin logins",
            &ctx,
            &embedder,
            &CalibrationConfig::default(),
            10,
        )
        .unwrap();

        let incident = suggestions
            .iter()
            .find(|s| s.entity_type == "incident")
            .unwrap();
        assert!(incident.negative_demoted);
        assert_ne!(incident.tier, ConfidenceTier::AutoClassifyEligible);
        let control = suggestions
            .iter()
            .find(|s| s.entity_type == "control")
            .unwrap();
        assert!(!control.negative_demoted);
    }

    #[test]
    fn calibration_for_a_different_model_reads_as_uncalibrated() {
        let Ok(embedder) = mif_embed::Embedder::load() else {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        };
        let (catalog, config, packs) = fixture();
        let ctx = ResolveContext {
            topic: "sec",
            catalog: &catalog,
            config: &config,
            ontology_packs: &packs,
        };
        let cal = CalibrationConfig {
            calibrated: true,
            embedding_model: "some-other/model".to_string(),
            ..CalibrationConfig::default()
        };

        let suggestions = suggest_type("MFA on admin logins", &ctx, &embedder, &cal, 10).unwrap();
        assert!(suggestions.iter().all(|s| !s.calibrated));
    }
}
