//! `stamped-quantile-v1` threshold calibration (MIF ADR-020, PDD-2).
//!
//! Derives a corpus's [`CalibrationConfig`] from the labeled sample the
//! corpus already carries: stamped findings (`basis` declared/resolved and
//! `valid`) have a ground-truth `entity_type` on disk. Each sample scores
//! its finding's indexed text against its topic's allowed entity types;
//! the sweep then picks the loosest `(tier1_floor, tier1_margin)` whose
//! accepted top-1 predictions meet a target precision, and the loosest
//! `tier2_floor` above which the gold type still appears among the
//! candidates at a target rate.
//!
//! Deliberately simple v1: an empirical grid sweep, not conformal
//! prediction — conformal risk control is the intended recalibration
//! upgrade path once real `/ontology-review --enrich` outcomes exist to
//! calibrate against. Threshold values stay artifact data either way; no
//! scoring code ever hardcodes them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use mif_ontology::{CalibrationConfig, OntologyError};
use serde::Serialize;

use crate::error::MifRhError;
use crate::resolve::{Basis, MapRecord, ResolveContext};
use crate::suggest::{SUGGESTION_DEPTH, build_candidates, suggest_from_candidates};
use crate::{Finding, index_text, review::list_finding_files};

/// One labeled calibration sample: a stamped finding's scoring outcome.
///
/// Non-exhaustive: samples are produced by [`collect_topic_samples`], not
/// constructed downstream, and the field set grows as calibration learns
/// to measure more (this release added the gold/top-1 types) — downstream
/// crates read fields and must stay compilable across such additions.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize)]
pub struct CalibrationSample {
    /// The stamped finding.
    pub finding_id: String,
    /// The topic whose candidate set scored this sample.
    pub topic: String,
    /// The finding's stamped (ground-truth) entity type.
    pub entity_type_gold: String,
    /// The entity type the top candidate named.
    pub entity_type_top1: String,
    /// The top candidate's score.
    pub top1_score: f32,
    /// The top candidate's lead over the second-best; `None` when the
    /// topic offered no rival candidate. Mirrors `assign_tier`'s vacuous
    /// margin pass: a no-rival sample satisfies every margin gate.
    pub top1_margin: Option<f32>,
    /// Whether the top candidate names the finding's stamped entity type.
    pub top1_correct: bool,
    /// Whether the stamped type appears in the top candidates at all.
    pub gold_in_candidates: bool,
}

/// Options for a calibration run.
#[derive(Debug, Clone, Copy)]
pub struct CalibrateOptions {
    /// Minimum empirical top-1 precision the tier-1 gate must achieve.
    pub target_precision: f32,
    /// Minimum gold-in-candidates rate above `tier2_floor`.
    pub tier2_target: f32,
    /// Cap on the number of stamped samples used (deterministic,
    /// seed-keyed selection). `None` uses every stamped finding.
    pub sample: Option<usize>,
    /// Seed for the deterministic subsample selection.
    pub seed: u64,
}

impl Default for CalibrateOptions {
    fn default() -> Self {
        Self {
            target_precision: 0.95,
            tier2_target: 0.5,
            sample: None,
            seed: 0,
        }
    }
}

/// Collects calibration samples for one topic: every stamped, valid
/// record in its `ontology-map.json` whose finding file still exists.
///
/// Scoring uses uncalibrated defaults deliberately — the raw scores and
/// ranks are what calibration measures; the tiers assigned during
/// collection are discarded.
///
/// # Errors
///
/// Returns [`MifRhError`] if the findings directory cannot be listed, a
/// finding fails to parse, or embedding fails.
pub fn collect_topic_samples(
    reports_dir: &Path,
    ctx: &ResolveContext<'_>,
    embedder: &mif_embed::Embedder,
) -> Result<Vec<CalibrationSample>, MifRhError> {
    let map_path = reports_dir.join(ctx.topic).join("ontology-map.json");
    let contents = match std::fs::read_to_string(&map_path) {
        Ok(contents) => contents,
        // Never reviewed — nothing stamped to learn from.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        // Any other read failure (permissions, I/O) would silently drop a
        // whole topic's labeled samples and bias the calibration — fail loud.
        Err(source) => {
            return Err(MifRhError::Io {
                path: map_path.display().to_string(),
                source,
            });
        },
    };
    let records: Vec<MapRecord> =
        serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
            path: map_path.display().to_string(),
            source,
        })?;

    let findings_dir = reports_dir.join(ctx.topic).join("findings");
    if !findings_dir.is_dir() {
        return Ok(Vec::new());
    }

    let neutral = CalibrationConfig::default();
    // One embedding pass over the topic's candidate documents, reused for
    // every finding below — per-finding re-embedding would cost
    // O(findings x types) forward passes.
    let candidates = build_candidates(ctx, embedder, &neutral)?;
    let mut samples = Vec::new();
    for file in list_finding_files(&findings_dir)? {
        let Ok(finding) = Finding::load(&file) else {
            continue; // gap findings are review's concern, not calibration's
        };
        let Some(record) = records.iter().find(|r| r.finding_id == finding.id) else {
            continue;
        };
        let stamped = record.valid && matches!(record.basis, Basis::Declared | Basis::Resolved);
        let Some(gold) = record.entity_type.as_deref() else {
            continue;
        };
        if !stamped {
            continue;
        }

        let query = index_text(&finding);
        if query.is_empty() {
            continue;
        }
        let query_vector = embedder.embed(&query)?;
        let ranked =
            suggest_from_candidates(&query_vector, &candidates, &neutral, SUGGESTION_DEPTH);
        let Some(top) = ranked.first() else {
            continue; // no scorable entity types for this topic
        };
        samples.push(CalibrationSample {
            finding_id: finding.id.clone(),
            topic: ctx.topic.to_string(),
            entity_type_gold: gold.to_string(),
            entity_type_top1: top.entity_type.clone(),
            top1_score: top.score,
            top1_margin: top.margin,
            top1_correct: top.entity_type == gold,
            gold_in_candidates: ranked.iter().any(|c| c.entity_type == gold),
        });
    }
    Ok(samples)
}

/// Deterministically caps `samples` to `opts.sample` entries, keyed by a
/// seed-mixed hash of each finding id (stable across runs and machines
/// for the same seed).
#[must_use]
pub fn subsample(
    mut samples: Vec<CalibrationSample>,
    opts: &CalibrateOptions,
) -> Vec<CalibrationSample> {
    let Some(cap) = opts.sample else {
        return samples;
    };
    if samples.len() <= cap {
        return samples;
    }
    samples.sort_by_key(|s| {
        let mut hasher = DefaultHasher::new();
        opts.seed.hash(&mut hasher);
        s.finding_id.hash(&mut hasher);
        hasher.finish()
    });
    samples.truncate(cap);
    samples
}

/// Sweeps the threshold grid over `samples`, producing a calibrated artifact.
///
/// Picks the loosest `(tier1_floor, tier1_margin)` (most accepted samples;
/// ties prefer the lower floor, then lower margin) whose accepted set has
/// top-1 precision `>= opts.target_precision`, and the lowest
/// `tier2_floor` at or below `tier1_floor` whose at-or-above sample set
/// keeps a gold-in-candidates rate `>= opts.tier2_target` (the bands
/// collapse to `tier1_floor` when no in-band floor qualifies).
///
/// # Errors
///
/// Returns [`OntologyError::CalibrationInvalid`] (wrapped in
/// [`MifRhError::Ontology`]) when `samples` is empty or no grid point
/// meets the precision target — an uncalibratable corpus must fail loud,
/// not silently emit thresholds that mean nothing.
pub fn sweep(
    samples: &[CalibrationSample],
    opts: &CalibrateOptions,
    artifact_path: &Path,
) -> Result<CalibrationConfig, MifRhError> {
    let invalid = |detail: String| {
        MifRhError::from(OntologyError::CalibrationInvalid {
            path: artifact_path.display().to_string(),
            detail,
        })
    };
    if samples.is_empty() {
        return Err(invalid(
            "no stamped, valid findings with scorable entity types to calibrate from — \
             review and stamp findings first"
                .to_string(),
        ));
    }

    // Grid in integer hundredths: exact iteration, exact tie-breaks. The
    // floor grid starts at 0 so a weak-scoring (but consistently correct)
    // corpus still calibrates — a low calibrated floor is an honest
    // statement about that corpus, not a failure.
    let mut best: Option<(usize, u8, u8)> = None; // (accepted, floor_pct, margin_pct)
    for floor_pct in 0..=95_u8 {
        for margin_pct in 0..=20_u8 {
            let Some(accepted) =
                accepted_meeting_target(samples, floor_pct, margin_pct, opts.target_precision)
            else {
                continue;
            };
            let candidate = (accepted, floor_pct, margin_pct);
            if best.is_none_or(|current| gate_is_better(candidate, current)) {
                best = Some(candidate);
            }
        }
    }
    let Some((_, tier1_floor_pct, tier1_margin_pct)) = best else {
        return Err(invalid(format!(
            "no (floor, margin) grid point reaches top-1 precision {} over {} samples — \
             enrich entity types (aliases/exemplars) or lower --target-precision",
            opts.target_precision,
            samples.len()
        )));
    };

    // tier2_floor: the lowest grid floor, at or below tier1_floor, whose
    // at-or-above set still finds the gold type among the candidates at
    // the target rate. The rate is NOT monotone in the floor (adding
    // low-score samples can push it below target at one floor and back
    // above at a lower one), so only floors within the band are scanned —
    // clamping a higher passing floor down to tier1_floor would fabricate
    // a floor whose own rate was never measured against the target. If no
    // in-band floor passes, the bands collapse (tier2_floor ==
    // tier1_floor): everything below tier 1 routes to tier 3, an honest
    // statement that the mid band offers no useful gold recall.
    let mut tier2_floor_pct = tier1_floor_pct;
    for floor_pct in 0..=tier1_floor_pct {
        let floor = f32::from(floor_pct) / 100.0;
        let (mut total, mut with_gold) = (0_usize, 0_usize);
        for s in samples.iter().filter(|s| s.top1_score >= floor) {
            total += 1;
            with_gold += usize::from(s.gold_in_candidates);
        }
        if total > 0 && ratio(with_gold, total) >= opts.tier2_target {
            tier2_floor_pct = floor_pct;
            break; // ascending scan: the first passing floor IS the lowest
        }
    }

    Ok(CalibrationConfig {
        tier1_floor: f32::from(tier1_floor_pct) / 100.0,
        tier1_margin: f32::from(tier1_margin_pct) / 100.0,
        tier2_floor: f32::from(tier2_floor_pct) / 100.0,
        calibrated: true,
        calibrated_at: Some(now_rfc3339()),
        sample_size: Some(u64::try_from(samples.len()).unwrap_or(u64::MAX)),
        method: Some("stamped-quantile-v1".to_string()),
        ..CalibrationConfig::default()
    })
}

/// Representative finding ids kept per confusion pair — enough to open a
/// handful of real findings during curation without dumping the corpus.
pub const CONFUSION_REPRESENTATIVES: usize = 5;

/// One confusable type pair from the stamped-sample set: `count` samples
/// stamped `gold` scored `top1` as their top candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfusionPair {
    /// The stamped (ground-truth) entity type the samples carry.
    pub gold: String,
    /// The entity type that incorrectly took top-1.
    pub top1: String,
    /// How many samples confused this pair.
    pub count: usize,
    /// Representative finding ids (lexicographically first
    /// [`CONFUSION_REPRESENTATIVES`] of the pair's samples), for opening
    /// real findings during curation.
    pub finding_ids: Vec<String>,
}

/// Whether any entity type across `packs` carries a non-empty curated
/// negative example.
///
/// `calibrate` evaluates this over the packs that actually scored the
/// swept samples (the union of `build_allowed` for topics represented in
/// the final sample set) and records the answer as the artifact's
/// `negatives_active` — an enabled-but-unbound pack, or one bound only to
/// topics that contributed no samples, never claims participation.
#[must_use]
pub fn packs_carry_negatives<'a>(packs: impl IntoIterator<Item = &'a crate::OntologyPack>) -> bool {
    packs
        .into_iter()
        .flat_map(|pack| &pack.entity_types)
        .any(|entity_type| {
            entity_type
                .negative_examples
                .iter()
                .any(|negative| !negative.trim().is_empty())
        })
}

/// The confusion-matrix artifact `calibrate --confusions` writes.
///
/// This grounds the human curation MIF ADR-020 mandates for
/// `negative_examples`. Derived data — consumers regenerate it from their
/// corpus and never commit it.
///
/// JSON shape (`version` is `"confusions-v1"`):
///
/// ```json
/// {
///   "version": "confusions-v1",
///   "sample_count": 128,
///   "pairs": [
///     {"gold": "curriculum", "top1": "title", "count": 7,
///      "finding_ids": ["f-012", "f-044", "f-051", "f-070", "f-093"]}
///   ]
/// }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct ConfusionReport {
    /// Artifact schema version, always `"confusions-v1"`.
    pub version: &'static str,
    /// How many calibration samples the pairs were derived from.
    pub sample_count: usize,
    /// Ranked confusable pairs, most frequent first.
    pub pairs: Vec<ConfusionPair>,
}

/// Derives the ranked confusion matrix from calibration samples.
///
/// Every incorrect top-1 groups under its `(gold, top1)` pair, ranked by
/// count descending, ties broken lexicographically by `(gold, top1)`.
/// Deterministic for a fixed sample set: representative ids are the
/// lexicographically first [`CONFUSION_REPRESENTATIVES`] of each pair.
#[must_use]
pub fn confusions(samples: &[CalibrationSample]) -> ConfusionReport {
    let mut by_pair: std::collections::BTreeMap<(&str, &str), Vec<&str>> =
        std::collections::BTreeMap::new();
    for s in samples.iter().filter(|s| !s.top1_correct) {
        by_pair
            .entry((s.entity_type_gold.as_str(), s.entity_type_top1.as_str()))
            .or_default()
            .push(s.finding_id.as_str());
    }
    let mut pairs: Vec<ConfusionPair> = by_pair
        .into_iter()
        .map(|((gold, top1), mut ids)| {
            ids.sort_unstable();
            let count = ids.len();
            ids.truncate(CONFUSION_REPRESENTATIVES);
            ConfusionPair {
                gold: gold.to_string(),
                top1: top1.to_string(),
                count,
                finding_ids: ids.into_iter().map(str::to_string).collect(),
            }
        })
        .collect();
    // BTreeMap iteration already yields (gold, top1) ascending, so the
    // stable sort's tie-break order is exactly the lexicographic one.
    pairs.sort_by_key(|p| std::cmp::Reverse(p.count));
    ConfusionReport {
        version: "confusions-v1",
        sample_count: samples.len(),
        pairs,
    }
}

/// `num / den` as `f32`; sample counts are far below `f32`'s exact-integer
/// range.
fn ratio(num: usize, den: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    {
        num as f32 / den as f32
    }
}

/// The accepted-sample count at one grid point, if that gate's top-1
/// precision meets `target`; `None` when it accepts nothing or misses the
/// target.
fn accepted_meeting_target(
    samples: &[CalibrationSample],
    floor_pct: u8,
    margin_pct: u8,
    target: f32,
) -> Option<usize> {
    let floor = f32::from(floor_pct) / 100.0;
    let margin = f32::from(margin_pct) / 100.0;
    let (mut accepted, mut correct) = (0_usize, 0_usize);
    for s in samples {
        // A no-rival sample passes any margin gate vacuously, exactly as
        // `assign_tier` treats a lone candidate at runtime — excluding it
        // here would let the artifact overstate the gate's real precision.
        let margin_ok = s.top1_margin.is_none_or(|m| m >= margin);
        if s.top1_score >= floor && margin_ok {
            accepted += 1;
            correct += usize::from(s.top1_correct);
        }
    }
    (accepted > 0 && ratio(correct, accepted) >= target).then_some(accepted)
}

/// Loosest-gate preference: more accepted samples wins; ties prefer the
/// lower floor, then the lower margin.
const fn gate_is_better(candidate: (usize, u8, u8), current: (usize, u8, u8)) -> bool {
    candidate.0 > current.0
        || (candidate.0 == current.0
            && (candidate.1 < current.1 || (candidate.1 == current.1 && candidate.2 < current.2)))
}

/// Current UTC time as RFC 3339 seconds (`YYYY-MM-DDTHH:MM:SSZ`).
fn now_rfc3339() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        CONFUSION_REPRESENTATIVES, CalibrateOptions, CalibrationSample, confusions, subsample,
        sweep,
    };

    fn sample(
        id: &str,
        score: f32,
        margin: f32,
        correct: bool,
        gold_in: bool,
    ) -> CalibrationSample {
        typed_sample(
            id,
            "gold-type",
            "other-type",
            score,
            margin,
            correct,
            gold_in,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn typed_sample(
        id: &str,
        gold: &str,
        top1: &str,
        score: f32,
        margin: f32,
        correct: bool,
        gold_in: bool,
    ) -> CalibrationSample {
        CalibrationSample {
            finding_id: id.to_string(),
            topic: "topic".to_string(),
            entity_type_gold: gold.to_string(),
            entity_type_top1: if correct {
                gold.to_string()
            } else {
                top1.to_string()
            },
            top1_score: score,
            top1_margin: Some(margin),
            top1_correct: correct,
            gold_in_candidates: gold_in,
        }
    }

    #[test]
    fn sweep_prefers_the_loosest_gate_meeting_the_precision_target() {
        // High scores are always correct; a mid score is wrong: the sweep
        // must pick a floor excluding the wrong one to hit precision 1.0.
        let samples = [
            sample("a", 0.90, 0.10, true, true),
            sample("b", 0.88, 0.09, true, true),
            sample("c", 0.60, 0.02, false, true),
        ];
        let cal = sweep(
            &samples,
            &CalibrateOptions {
                target_precision: 1.0,
                ..CalibrateOptions::default()
            },
            Path::new("test.json"),
        )
        .unwrap();

        assert!(cal.calibrated);
        // The gate must exclude the wrong sample (by floor OR margin — the
        // loosest-gate tie-break may pick either mechanism)...
        assert!(
            cal.tier1_floor > 0.60 || cal.tier1_margin > 0.02,
            "gate must exclude the wrong sample (floor {}, margin {})",
            cal.tier1_floor,
            cal.tier1_margin
        );
        // ...while both correct samples still pass it.
        assert!(cal.tier1_floor <= 0.88 && cal.tier1_margin <= 0.09);
        assert!(cal.tier2_floor <= cal.tier1_floor);
        assert_eq!(cal.method.as_deref(), Some("stamped-quantile-v1"));
        assert_eq!(cal.sample_size, Some(3));
    }

    #[test]
    fn no_rival_samples_pass_margin_gates_vacuously_matching_runtime() {
        // A no-rival sample (margin None) is auto-classify-eligible at
        // runtime whenever its score clears the floor, so the sweep must
        // count it toward every margin gate's precision rather than
        // exclude it and overstate what the gate delivers.
        let lone = CalibrationSample {
            finding_id: "lone".to_string(),
            topic: "topic".to_string(),
            entity_type_gold: "gold-type".to_string(),
            entity_type_top1: "gold-type".to_string(),
            top1_score: 0.90,
            top1_margin: None,
            top1_correct: true,
            gold_in_candidates: true,
        };
        let rivaled = sample("rivaled", 0.88, 0.10, true, true);
        let cal = sweep(
            &[lone, rivaled],
            &CalibrateOptions {
                target_precision: 1.0,
                ..CalibrateOptions::default()
            },
            Path::new("test.json"),
        )
        .unwrap();
        assert!(cal.calibrated);
        assert_eq!(cal.sample_size, Some(2));
    }

    #[test]
    fn sweep_fails_loud_on_an_empty_sample_set() {
        let error = sweep(&[], &CalibrateOptions::default(), Path::new("test.json")).unwrap_err();
        assert!(error.to_string().contains("no stamped"));
    }

    #[test]
    fn sweep_fails_loud_when_no_grid_point_reaches_the_target() {
        // Every top-1 is wrong: no gate can reach any positive precision.
        let samples = [
            sample("a", 0.90, 0.10, false, true),
            sample("b", 0.88, 0.09, false, false),
        ];
        let error = sweep(
            &samples,
            &CalibrateOptions::default(),
            Path::new("test.json"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("precision"));
    }

    #[test]
    fn confusions_rank_known_pairs_by_count_then_lexicographically() {
        // A fixture corpus with a known confusion structure: three
        // curriculum->title misses, one title->curriculum miss, one
        // correct sample that must not appear at all.
        let samples = [
            typed_sample("f-3", "curriculum", "title", 0.7, 0.05, false, true),
            typed_sample("f-1", "curriculum", "title", 0.6, 0.04, false, true),
            typed_sample("f-2", "curriculum", "title", 0.8, 0.06, false, true),
            typed_sample("f-4", "title", "curriculum", 0.5, 0.02, false, true),
            typed_sample("f-5", "title", "title", 0.9, 0.20, true, true),
        ];
        let report = confusions(&samples);

        assert_eq!(report.version, "confusions-v1");
        assert_eq!(report.sample_count, 5);
        assert_eq!(report.pairs.len(), 2);
        assert_eq!(report.pairs[0].gold, "curriculum");
        assert_eq!(report.pairs[0].top1, "title");
        assert_eq!(report.pairs[0].count, 3);
        // Representatives are the lexicographically first ids, not
        // insertion order.
        assert_eq!(report.pairs[0].finding_ids, ["f-1", "f-2", "f-3"]);
        assert_eq!(report.pairs[1].count, 1);
        assert_eq!(report.pairs[1].gold, "title");
    }

    #[test]
    fn confusions_are_deterministic_and_cap_representatives() {
        let build = |order: &[usize]| {
            order
                .iter()
                .map(|i| typed_sample(&format!("f-{i}"), "a", "b", 0.5, 0.0, false, true))
                .collect::<Vec<_>>()
        };
        // Same samples in two different orders must yield identical reports.
        let forward = confusions(&build(&[1, 2, 3, 4, 5, 6, 7, 8]));
        let reverse = confusions(&build(&[8, 7, 6, 5, 4, 3, 2, 1]));
        assert_eq!(forward.pairs, reverse.pairs);

        assert_eq!(forward.pairs[0].count, 8);
        assert_eq!(
            forward.pairs[0].finding_ids.len(),
            CONFUSION_REPRESENTATIVES
        );
        assert_eq!(forward.pairs[0].finding_ids[0], "f-1");
    }

    #[test]
    fn confusions_on_an_all_correct_corpus_are_empty_but_versioned() {
        let samples = [sample("a", 0.9, 0.1, true, true)];
        let report = confusions(&samples);
        assert_eq!(report.version, "confusions-v1");
        assert_eq!(report.sample_count, 1);
        assert!(report.pairs.is_empty());
    }

    #[test]
    fn subsample_is_deterministic_and_seed_sensitive() {
        let build = || {
            (0..20)
                .map(|i| sample(&format!("f-{i}"), 0.5, 0.0, true, true))
                .collect::<Vec<_>>()
        };
        let opts_a = CalibrateOptions {
            sample: Some(5),
            seed: 1,
            ..CalibrateOptions::default()
        };
        let first = subsample(build(), &opts_a);
        let second = subsample(build(), &opts_a);
        assert_eq!(
            first.iter().map(|s| &s.finding_id).collect::<Vec<_>>(),
            second.iter().map(|s| &s.finding_id).collect::<Vec<_>>()
        );
        assert_eq!(first.len(), 5);

        let opts_b = CalibrateOptions {
            sample: Some(5),
            seed: 2,
            ..CalibrateOptions::default()
        };
        let other_seed = subsample(build(), &opts_b);
        assert_ne!(
            first.iter().map(|s| &s.finding_id).collect::<Vec<_>>(),
            other_seed.iter().map(|s| &s.finding_id).collect::<Vec<_>>()
        );
    }
}
