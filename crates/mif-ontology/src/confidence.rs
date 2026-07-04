//! The two-threshold, three-tier confidence-score policy for
//! embedding-based entity-type classification (MIF ADR-020).
//!
//! Two calibrated thresholds partition a classification score into three
//! action bands — [`ConfidenceTier::AutoClassifyEligible`],
//! [`ConfidenceTier::FlagForReview`], [`ConfidenceTier::TriggerExpansion`] —
//! with a top-1/top-2 margin check gating the top tier (TAC-KBP's
//! two-parameter entity-linking pattern: a high-but-ambiguous top score must
//! not auto-classify). Threshold values are **calibrated, recalibratable
//! data**, never hardcoded constants: they load from a per-corpus
//! calibration artifact ([`CalibrationConfig::load_or_default`]), and the
//! built-in defaults used when no artifact exists are explicitly marked
//! `calibrated: false` so consumers can see the scores are ungoverned.
//! Conformal-prediction-based recalibration is the intended upgrade path
//! for producing the artifact; this module only defines its shape and the
//! pure tier-assignment logic.
//!
//! No embedding execution happens here — callers supply scores/vectors,
//! keeping this crate free of any inference dependency.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::OntologyError;

/// The embedding model the built-in default thresholds were sketched against.
///
/// A calibration artifact naming a different model than the one a consumer
/// actually runs should be treated as uncalibrated by that consumer.
pub const DEFAULT_EMBEDDING_MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// One of the three action bands a classification score falls into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceTier {
    /// Tier 1: the score clears the calibrated floor AND holds a clear
    /// margin over the second-best candidate. "Eligible" is deliberate —
    /// no tier ever auto-writes a type; a confirming agent/human action is
    /// always required.
    AutoClassifyEligible,
    /// Tier 2: mid-band — route to a human-reviewable queue rather than
    /// silently discarding or auto-accepting.
    FlagForReview,
    /// Tier 3: low-band — a candidate signal for ontology expansion, but
    /// only once repeated, mutually-similar misses cluster across runs;
    /// never from a single low-confidence miss.
    TriggerExpansion,
}

/// Knobs for the tier-3 miss-clustering criterion. All configurable,
/// carried in the same calibration artifact as the score thresholds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExpansionConfig {
    /// Minimum pairwise cosine similarity every pair of members of one
    /// cluster must satisfy (mutual, not chained).
    pub cluster_similarity: f32,
    /// Minimum cluster size before it surfaces as an expansion candidate.
    pub min_cluster_size: usize,
    /// Minimum number of distinct runs the cluster's members must span —
    /// "repeated across runs", not one bad batch.
    pub min_distinct_runs: usize,
}

impl Default for ExpansionConfig {
    fn default() -> Self {
        Self {
            cluster_similarity: 0.80,
            min_cluster_size: 3,
            min_distinct_runs: 2,
        }
    }
}

/// The calibration artifact: score thresholds plus their provenance.
///
/// Lives as derived, per-corpus data (conventionally
/// `reports/_meta/confidence-calibration.json` in a research-harness
/// corpus), never as authored configuration — see
/// [`CalibrationConfig::load_or_default`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationConfig {
    /// The embedding model these thresholds were calibrated for.
    pub embedding_model: String,
    /// Tier-1 floor: the minimum top-candidate score for auto-classify
    /// eligibility.
    pub tier1_floor: f32,
    /// Tier-1 margin: the minimum lead the top candidate must hold over
    /// the second-best candidate.
    pub tier1_margin: f32,
    /// Tier-2 floor: the minimum score for flag-for-review; below it a
    /// score is a trigger-expansion miss.
    pub tier2_floor: f32,
    /// Whether these values came from a real calibration run (`true`) or
    /// are the built-in uncalibrated defaults (`false`).
    pub calibrated: bool,
    /// When the calibration run happened (RFC 3339), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibrated_at: Option<String>,
    /// How many labeled samples the calibration run used, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<u64>,
    /// The calibration method identifier (e.g. `stamped-quantile-v1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Tier-3 clustering knobs.
    #[serde(default)]
    pub expansion: ExpansionConfig,
}

impl Default for CalibrationConfig {
    /// Conservative, precedent-anchored placeholder values, explicitly
    /// marked uncalibrated. These are default *artifact values* a real
    /// `calibrate` run replaces — scoring code never hardcodes them.
    fn default() -> Self {
        Self {
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
            tier1_floor: 0.85,
            tier1_margin: 0.05,
            tier2_floor: 0.60,
            calibrated: false,
            calibrated_at: None,
            sample_size: None,
            method: None,
            expansion: ExpansionConfig::default(),
        }
    }
}

impl CalibrationConfig {
    /// Loads the calibration artifact at `path`, or returns the built-in
    /// uncalibrated defaults if no file exists there.
    ///
    /// A *missing* artifact is normal (calibration has simply never run);
    /// a *present but unusable* one is not — malformed JSON, non-finite or
    /// mis-ordered thresholds — and errors rather than silently falling
    /// back, so a corrupted artifact cannot masquerade as "never
    /// calibrated".
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::CalibrationInvalid`] if the file exists but
    /// cannot be read, parsed, or fails [`Self::validate`].
    pub fn load_or_default(path: &Path) -> Result<Self, OntologyError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let invalid = |detail: String| OntologyError::CalibrationInvalid {
            path: path.display().to_string(),
            detail,
        };
        let contents = std::fs::read_to_string(path).map_err(|e| invalid(e.to_string()))?;
        let config: Self = serde_json::from_str(&contents).map_err(|e| invalid(e.to_string()))?;
        config.validate().map_err(invalid)?;
        Ok(config)
    }

    /// Checks the threshold values are usable: finite, within `[-1, 1]`
    /// (cosine range), a non-negative margin, and `tier2_floor <=
    /// tier1_floor` (the bands must nest).
    ///
    /// # Errors
    ///
    /// Returns a human-readable description of the first violation.
    pub fn validate(&self) -> Result<(), String> {
        for (label, value) in [
            ("tier1_floor", self.tier1_floor),
            ("tier1_margin", self.tier1_margin),
            ("tier2_floor", self.tier2_floor),
        ] {
            if !value.is_finite() {
                return Err(format!("{label} is not a finite number"));
            }
        }
        if !(-1.0..=1.0).contains(&self.tier1_floor) || !(-1.0..=1.0).contains(&self.tier2_floor) {
            return Err("tier floors must be within the cosine range [-1, 1]".to_string());
        }
        if self.tier1_margin < 0.0 {
            return Err("tier1_margin must be non-negative".to_string());
        }
        if self.tier2_floor > self.tier1_floor {
            return Err(format!(
                "tier2_floor ({}) must not exceed tier1_floor ({})",
                self.tier2_floor, self.tier1_floor
            ));
        }
        Ok(())
    }
}

/// Assigns the confidence tier for one ranked candidate.
///
/// Only the top-ranked candidate (`rank == 0`) can ever be
/// [`ConfidenceTier::AutoClassifyEligible`], and only when its score clears
/// `cal.tier1_floor` AND leads `second_best` by at least `cal.tier1_margin`
/// (a lone candidate has no rival, so the margin check passes vacuously).
/// Any candidate at or above `cal.tier2_floor` otherwise flags for review;
/// everything below is an expansion-trigger miss.
#[must_use]
pub fn assign_tier(
    rank: usize,
    score: f32,
    second_best: Option<f32>,
    cal: &CalibrationConfig,
) -> ConfidenceTier {
    if rank == 0
        && score >= cal.tier1_floor
        && second_best.is_none_or(|second| score - second >= cal.tier1_margin)
    {
        return ConfidenceTier::AutoClassifyEligible;
    }
    // Everything that is not the margin-cleared top candidate caps at
    // flag-for-review, however high its raw score — a high score at rank 1,
    // or a high-but-ambiguous top score, is precisely what the margin check
    // exists to demote.
    if score >= cal.tier2_floor {
        ConfidenceTier::FlagForReview
    } else {
        ConfidenceTier::TriggerExpansion
    }
}

/// Bands a score by the two floors alone, with no rank/margin semantics.
///
/// For score streams that are not a classification decision (e.g.
/// `find_similar`'s duplicate-confidence annotation, where the top band
/// reads as "near-duplicate candidate", not "safe to auto-classify").
#[must_use]
pub fn band_by_score(score: f32, cal: &CalibrationConfig) -> ConfidenceTier {
    if score >= cal.tier1_floor {
        ConfidenceTier::AutoClassifyEligible
    } else if score >= cal.tier2_floor {
        ConfidenceTier::FlagForReview
    } else {
        ConfidenceTier::TriggerExpansion
    }
}

/// Greedy mutual-similarity clustering over caller-supplied vectors.
///
/// Returns clusters as index lists into `vectors`; every *pair* of members
/// within one cluster has cosine similarity `>= threshold` (mutual — a
/// chain `a~b~c` where `a` and `c` are dissimilar does NOT form one
/// cluster). Singleton clusters are omitted. Order is deterministic:
/// clusters seed in input order and members join in input order.
///
/// Size/recurrence gating ([`ExpansionConfig::min_cluster_size`],
/// [`ExpansionConfig::min_distinct_runs`]) is the caller's job — run
/// identity lives with the caller's data, not here. Brute-force `O(n^2)`
/// pairwise scoring, adequate at the corpus scales this workspace targets
/// (the same argument `mif-rh`'s index makes for brute-force search).
#[must_use]
pub fn cluster_by_mutual_similarity(vectors: &[Vec<f32>], threshold: f32) -> Vec<Vec<usize>> {
    let mut assigned = vec![false; vectors.len()];
    let mut clusters = Vec::new();
    for seed in 0..vectors.len() {
        if assigned[seed] {
            continue;
        }
        let mut members = vec![seed];
        for candidate in (seed + 1)..vectors.len() {
            if assigned[candidate] {
                continue;
            }
            let mutual = members
                .iter()
                .all(|&m| cosine_similarity(&vectors[m], &vectors[candidate]) >= threshold);
            if mutual {
                members.push(candidate);
            }
        }
        if members.len() > 1 {
            for &m in &members {
                assigned[m] = true;
            }
            clusters.push(members);
        }
    }
    clusters
}

/// Cosine similarity between two vectors; `0.0` for mismatched dimensions
/// or zero-magnitude inputs.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::{
        CalibrationConfig, ConfidenceTier, assign_tier, band_by_score, cluster_by_mutual_similarity,
    };

    fn cal() -> CalibrationConfig {
        CalibrationConfig::default()
    }

    #[test]
    fn rank_zero_with_floor_and_margin_is_auto_classify_eligible() {
        let tier = assign_tier(0, 0.90, Some(0.70), &cal());
        assert_eq!(tier, ConfidenceTier::AutoClassifyEligible);
    }

    #[test]
    fn rank_zero_exactly_at_floor_and_margin_is_auto_classify_eligible() {
        let tier = assign_tier(0, 0.85, Some(0.80), &cal());
        assert_eq!(tier, ConfidenceTier::AutoClassifyEligible);
    }

    #[test]
    fn rank_zero_below_floor_is_not_tier_one() {
        let tier = assign_tier(0, 0.84, Some(0.10), &cal());
        assert_eq!(tier, ConfidenceTier::FlagForReview);
    }

    #[test]
    fn rank_zero_failing_the_margin_is_demoted_to_review() {
        // High top score, but ambiguous between two candidates.
        let tier = assign_tier(0, 0.90, Some(0.88), &cal());
        assert_eq!(tier, ConfidenceTier::FlagForReview);
    }

    #[test]
    fn a_lone_candidate_passes_the_margin_vacuously() {
        let tier = assign_tier(0, 0.90, None, &cal());
        assert_eq!(tier, ConfidenceTier::AutoClassifyEligible);
    }

    #[test]
    fn non_zero_rank_is_never_tier_one() {
        let tier = assign_tier(1, 0.99, Some(0.10), &cal());
        assert_eq!(tier, ConfidenceTier::FlagForReview);
    }

    #[test]
    fn below_tier_two_floor_is_trigger_expansion() {
        let tier = assign_tier(0, 0.30, Some(0.10), &cal());
        assert_eq!(tier, ConfidenceTier::TriggerExpansion);
    }

    #[test]
    fn band_by_score_ignores_margin_semantics() {
        assert_eq!(
            band_by_score(0.99, &cal()),
            ConfidenceTier::AutoClassifyEligible
        );
        assert_eq!(band_by_score(0.70, &cal()), ConfidenceTier::FlagForReview);
        assert_eq!(
            band_by_score(0.10, &cal()),
            ConfidenceTier::TriggerExpansion
        );
    }

    #[test]
    fn load_or_default_on_a_missing_file_is_uncalibrated_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = CalibrationConfig::load_or_default(&dir.path().join("absent.json")).unwrap();
        assert!(!config.calibrated);
        assert_eq!(config, CalibrationConfig::default());
    }

    #[test]
    fn load_or_default_roundtrips_a_written_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("confidence-calibration.json");
        let written = CalibrationConfig {
            tier1_floor: 0.91,
            calibrated: true,
            method: Some("stamped-quantile-v1".to_string()),
            ..CalibrationConfig::default()
        };
        std::fs::write(&path, serde_json::to_string_pretty(&written).unwrap()).unwrap();

        let loaded = CalibrationConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded, written);
    }

    #[test]
    fn load_or_default_rejects_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("confidence-calibration.json");
        std::fs::write(&path, "{ not json").unwrap();
        let error = CalibrationConfig::load_or_default(&path).unwrap_err();
        assert!(matches!(
            error,
            crate::OntologyError::CalibrationInvalid { .. }
        ));
    }

    #[test]
    fn validate_rejects_mis_ordered_floors() {
        let config = CalibrationConfig {
            tier2_floor: 0.95, // above tier1_floor 0.85
            ..CalibrationConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_non_finite_and_out_of_range_values() {
        for config in [
            CalibrationConfig {
                tier1_floor: f32::NAN,
                ..CalibrationConfig::default()
            },
            CalibrationConfig {
                tier1_margin: -0.1,
                ..CalibrationConfig::default()
            },
            CalibrationConfig {
                tier1_floor: 1.5,
                ..CalibrationConfig::default()
            },
        ] {
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn tier_ordering_is_monotone_in_score_at_fixed_rank_and_margin() {
        // A coarse sweep standing in for a property test: as score rises
        // with a fixed comfortable margin, the tier never gets less
        // confident.
        let cal = cal();
        let rank_of = |tier: ConfidenceTier| match tier {
            ConfidenceTier::TriggerExpansion => 0,
            ConfidenceTier::FlagForReview => 1,
            ConfidenceTier::AutoClassifyEligible => 2,
        };
        let mut previous = 0;
        for step in 0_i16..=200 {
            let score = f32::from(step) / 100.0 - 1.0;
            let tier = assign_tier(0, score, Some(score - 0.2), &cal);
            let current = rank_of(tier);
            assert!(current >= previous, "tier regressed at score {score}");
            previous = current;
        }
    }

    #[test]
    fn clusters_require_mutual_similarity_not_chains() {
        // a ~ b (cos ≈ 0.9) and b ~ c (cos ≈ 0.44), but a ⊥ c (cos = 0):
        // c must not chain into the {a, b} cluster.
        let a = vec![1.0, 0.0];
        let b = vec![0.9, 0.435_889_9]; // unit vector, cos(a, b) ≈ 0.9
        let c = vec![0.0, 1.0];
        let clusters = cluster_by_mutual_similarity(&[a, b, c], 0.4);
        assert_eq!(clusters, vec![vec![0, 1]]);
    }

    #[test]
    fn identical_vectors_cluster_and_singletons_are_omitted() {
        let clusters = cluster_by_mutual_similarity(
            &[
                vec![1.0, 0.0],
                vec![1.0, 0.0],
                vec![0.0, 1.0], // lone outlier
            ],
            0.95,
        );
        assert_eq!(clusters, vec![vec![0, 1]]);
    }

    #[test]
    fn empty_input_yields_no_clusters() {
        assert!(cluster_by_mutual_similarity(&[], 0.8).is_empty());
    }
}
