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
use crate::suggest::suggest_type;
use crate::{Finding, index_text, review::list_finding_files};

/// How many ranked candidates "gold appears among the candidates" checks.
const GOLD_RECALL_DEPTH: usize = 5;

/// One labeled calibration sample: a stamped finding's scoring outcome.
#[derive(Debug, Clone, Serialize)]
pub struct CalibrationSample {
    /// The stamped finding.
    pub finding_id: String,
    /// The top candidate's score.
    pub top1_score: f32,
    /// The top candidate's lead over the second-best (`0.0` when no rival).
    pub top1_margin: f32,
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
    let Ok(contents) = std::fs::read_to_string(&map_path) else {
        return Ok(Vec::new()); // never reviewed — nothing stamped to learn from
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
        let candidates = suggest_type(&query, ctx, embedder, &neutral, GOLD_RECALL_DEPTH)?;
        let Some(top) = candidates.first() else {
            continue; // no scorable entity types for this topic
        };
        samples.push(CalibrationSample {
            finding_id: finding.id.clone(),
            top1_score: top.score,
            top1_margin: top.margin.unwrap_or(0.0),
            top1_correct: top.entity_type == gold,
            gold_in_candidates: candidates.iter().any(|c| c.entity_type == gold),
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
/// top-1 precision `>= opts.target_precision`, and the lowest `tier2_floor`
/// such that samples at or above it keep a gold-in-candidates rate
/// `>= opts.tier2_target` (clamped to `tier1_floor`).
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

    // tier2_floor: the lowest grid floor whose at-or-above set still finds
    // the gold type among the candidates at the target rate.
    let mut tier2_floor_pct = tier1_floor_pct;
    for floor_pct in (0..=95_u8).rev() {
        let floor = f32::from(floor_pct) / 100.0;
        let at_or_above = samples.iter().filter(|s| s.top1_score >= floor);
        let (mut total, mut with_gold) = (0_usize, 0_usize);
        for s in at_or_above {
            total += 1;
            with_gold += usize::from(s.gold_in_candidates);
        }
        if total == 0 {
            continue;
        }
        if ratio(with_gold, total) >= opts.tier2_target {
            tier2_floor_pct = floor_pct.min(tier1_floor_pct);
        } else {
            break; // rates only degrade as the floor keeps dropping
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
        if s.top1_score >= floor && s.top1_margin >= margin {
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

/// Current UTC time as RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`), derived from the
/// system clock without a date-time dependency (civil-from-days per Howard
/// Hinnant's algorithm).
fn now_rfc3339() -> String {
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(epoch_secs / 86_400).unwrap_or(0);
    let secs_of_day = epoch_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Converts days since 1970-01-01 to a civil (year, month, day) date.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // day/month fit u8 by construction
const fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CalibrateOptions, CalibrationSample, civil_from_days, subsample, sweep};

    fn sample(
        id: &str,
        score: f32,
        margin: f32,
        correct: bool,
        gold_in: bool,
    ) -> CalibrationSample {
        CalibrationSample {
            finding_id: id.to_string(),
            top1_score: score,
            top1_margin: margin,
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

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // 2024-01-01
        assert_eq!(civil_from_days(20_638), (2026, 7, 4)); // 2026-07-04
    }
}
