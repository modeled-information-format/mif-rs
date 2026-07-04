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
//! Suggestions are hypotheses, never stamps: nothing here writes to
//! `reports/`, and even a [`ConfidenceTier::AutoClassifyEligible`]
//! candidate requires a confirming agent/human action to become a
//! finding's `entity_type`. This module lives in the hypothesis layer of
//! this crate's determinism boundary (see the crate doc) — `resolve`/
//! `review` never call it.

use mif_ontology::{CalibrationConfig, ConfidenceTier, assign_tier};
use serde::{Deserialize, Serialize};

use crate::error::MifRhError;
use crate::index::cosine_similarity;
use crate::resolve::{ResolveContext, build_allowed};

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
}

/// Suggests candidate entity types for `text` against `ctx.topic`'s
/// allowed ontologies, ranked by similarity and annotated with confidence
/// tiers.
///
/// Entity types with no positive embedding signal (no description, aliases,
/// or exemplars) are skipped. Tier assignment happens over the full ranked
/// candidate set *before* truncation to `limit`, so the top candidate's
/// margin is always measured against the true second-best, not the best
/// survivor of an early cut.
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
    let allowed = build_allowed(ctx)?;
    let query_vector = embedder.embed(text)?;

    // The calibration only governs scores produced by the model actually
    // in use; an artifact naming any other model reads as uncalibrated.
    let calibrated = cal.governs(mif_embed::MODEL_ID);

    let mut scored: Vec<(String, String, f32)> = Vec::new();
    for pack in &allowed {
        for entity_type in &pack.entity_types {
            let Some(doc) = entity_type.embedding_doc() else {
                continue;
            };
            let candidate_vector = embedder.embed(&doc)?;
            let score = cosine_similarity(&query_vector, &candidate_vector);
            scored.push((entity_type.name.clone(), pack.id.clone(), score));
        }
    }

    // Total order: score desc, then ontology id, then type name — exact
    // score ties (identical embedding docs across packs) must rank
    // deterministically, including which twin sits at rank 0 carrying the
    // margin, since build_allowed's pack order is hash-map-dependent.
    scored.sort_by(|a, b| {
        b.2.total_cmp(&a.2)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.0.cmp(&b.0))
    });
    let second_best = scored.get(1).map(|(_, _, score)| *score);

    let mut suggestions: Vec<TypeSuggestion> = scored
        .into_iter()
        .enumerate()
        .map(|(rank, (entity_type, ontology_id, score))| TypeSuggestion {
            entity_type,
            ontology_id,
            score,
            tier: assign_tier(rank, score, second_best, cal),
            margin: (rank == 0)
                .then(|| second_best.map(|second| score - second))
                .flatten(),
            calibrated,
        })
        .collect();
    suggestions.truncate(limit);
    Ok(suggestions)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mif_ontology::CalibrationConfig;

    use super::suggest_type;
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
