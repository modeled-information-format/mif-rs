//! Tier-2 scored-suggestion queue and tier-3 expansion candidates.
//!
//! Routing for MIF ADR-020's lower two tiers, per this workspace's
//! "2b-routing, 2a-pipeline" decision: the queue and miss store are
//! purpose-built for *scored candidate lists* (rht's `--followup` backlog
//! carries one unscored entity type per entry and is atomically rebuilt
//! every review — structurally wrong for either job), while the surfaces
//! that consume them are rht's existing ones (`/ontology-review --enrich`
//! reads the queue; `author-ontology.sh --from-clusters` mines the
//! expansion candidates).
//!
//! Everything here is written by `mif-rh-cli` paths only — `mif-rh-mcp`
//! stays read-only — and nothing here ever writes a finding's
//! `entity_type`: confirming or rejecting a queue entry is the human/agent
//! `--enrich` step's job (PDD-1).

use std::collections::HashSet;
use std::path::Path;

use mif_ontology::ExpansionConfig;
use serde::{Deserialize, Serialize};

use crate::error::MifRhError;
use crate::index::Miss;
use crate::suggest::TypeSuggestion;

/// The status every fresh queue entry starts in. Any other status is a
/// human/agent verdict and is never overwritten by a re-suggestion.
pub const STATUS_PENDING: &str = "pending";

/// One queued scored suggestion: a finding that is not durably stamped,
/// with its ranked, tier-annotated candidate list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionEntry {
    /// The finding's id.
    pub finding_id: String,
    /// The finding's file path exactly as the producing review listed it:
    /// absolute when the review ran with an absolute `--reports-dir`,
    /// otherwise relative to that review's working directory. Not
    /// normalized to repo-relative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Why the finding needed a suggestion: its followup basis
    /// (`"discovery"`, `"untyped"`, `"gap"`, ...).
    pub basis: String,
    /// The run that produced (or last refreshed) this entry.
    pub run_id: String,
    /// Ranked, tier-annotated candidates.
    pub candidates: Vec<TypeSuggestion>,
    /// Review status: [`STATUS_PENDING`] until a human/agent confirms or
    /// rejects via `/ontology-review --enrich`. Free-form beyond
    /// `pending` — any non-pending value is preserved verbatim on upsert.
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    STATUS_PENDING.to_string()
}

/// One topic's suggestion queue (`reports/_meta/suggestions/<topic>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionQueue {
    /// The topic this queue belongs to.
    pub topic: String,
    /// Queue entries, sorted by `finding_id`.
    pub entries: Vec<SuggestionEntry>,
}

/// Upserts `fresh` entries into the topic queue at `path`.
///
/// Preserves human verdicts: an existing entry is replaced only while its
/// status is still [`STATUS_PENDING`]; confirmed/rejected entries are kept
/// verbatim. Existing entries not re-suggested this run are also kept
/// (their findings may simply not have been part of this run's scope).
///
/// A missing queue file starts empty; a present-but-unparsable one is an
/// explicit error, never silently reset — it may carry human verdicts.
///
/// Two contract notes for consumers:
///
/// - **Verdict writers must not race this upsert.** The atomic rename
///   prevents torn files, not lost updates: a verdict written between this
///   function's read and its rename is clobbered. `review --suggest` runs
///   under the review lock; any surface writing verdicts (rht's
///   `/ontology-review --enrich`, a human editor) must not run concurrently
///   with a suggesting review.
/// - **The queue only grows.** Entries not re-suggested are kept even when
///   pending — a `--topic`-scoped run legitimately re-suggests only a
///   subset, and this function cannot tell scope from staleness. Pending
///   entries whose findings have since been stamped or deleted therefore
///   accumulate until a consumer prunes them; treat `pending` as "not yet
///   reviewed", not "still applicable".
///
/// # Errors
///
/// Returns [`MifRhError::Json`] if an existing queue file cannot be
/// parsed, or [`MifRhError::Io`]/[`MifRhError::JsonSerialize`] if the
/// updated queue cannot be written.
pub fn upsert_suggestions(
    path: &Path,
    topic: &str,
    fresh: Vec<SuggestionEntry>,
) -> Result<SuggestionQueue, MifRhError> {
    let mut queue = if path.exists() {
        let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let queue = serde_json::from_str::<SuggestionQueue>(&contents).map_err(|source| {
            MifRhError::Json {
                path: path.display().to_string(),
                source,
            }
        })?;
        // A queue file recorded for another topic (copied, renamed, or a
        // wrong-path caller) must not silently absorb this topic's entries.
        if queue.topic != topic {
            return Err(MifRhError::QueueTopicMismatch {
                path: path.display().to_string(),
                expected: topic.to_string(),
                found: queue.topic,
            });
        }
        queue
    } else {
        SuggestionQueue {
            topic: topic.to_string(),
            entries: Vec::new(),
        }
    };

    for entry in fresh {
        match queue
            .entries
            .iter_mut()
            .find(|existing| existing.finding_id == entry.finding_id)
        {
            Some(existing) if existing.status == STATUS_PENDING => *existing = entry,
            Some(_) => {}, // a human verdict — never overwritten
            None => queue.entries.push(entry),
        }
    }
    queue
        .entries
        .sort_by(|a, b| a.finding_id.cmp(&b.finding_id));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| MifRhError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    crate::write_json_atomic(path, &queue)?;
    Ok(queue)
}

/// One member of an expansion-candidate cluster.
#[derive(Debug, Clone, Serialize)]
pub struct ClusterMember {
    /// The finding whose classification missed.
    pub finding_id: String,
    /// The finding's topic.
    pub topic: String,
    /// The query text that missed.
    pub content: String,
    /// The run that recorded the miss.
    pub run_id: String,
}

/// One ontology-expansion candidate: a recurring cluster of
/// mutually-similar tier-3 misses.
#[derive(Debug, Clone, Serialize)]
pub struct ExpansionCandidate {
    /// Distinct findings in the cluster.
    pub size: usize,
    /// Distinct runs the cluster's misses span.
    pub runs: usize,
    /// The clustered misses.
    pub members: Vec<ClusterMember>,
}

/// Clusters recorded misses into ontology-expansion candidates.
///
/// Greedy mutual-similarity clustering
/// ([`mif_ontology::cluster_by_mutual_similarity`]) over the stored
/// vectors, surfacing only clusters with at least `cfg.min_cluster_size`
/// **distinct findings** spanning at least `cfg.min_distinct_runs`
/// distinct runs — recurrence across runs, never a single low-confidence
/// miss (MIF ADR-020, tier 3).
///
/// Clustering ignores topic boundaries deliberately: a concept missing a
/// type across several topics is exactly the cross-topic signal worth
/// surfacing, and each member carries its `topic` so reviewers see the
/// span. Callers should filter `misses` to one embedding model first
/// ([`Miss::model`]) — vectors from different models do not share a space.
#[must_use]
pub fn expansion_candidates(misses: &[Miss], cfg: &ExpansionConfig) -> Vec<ExpansionCandidate> {
    let vectors: Vec<Vec<f32>> = misses.iter().map(|m| m.vector.clone()).collect();
    let clusters = mif_ontology::cluster_by_mutual_similarity(&vectors, cfg.cluster_similarity);

    clusters
        .into_iter()
        .filter_map(|indices| {
            let members: Vec<ClusterMember> = indices
                .iter()
                .map(|&i| ClusterMember {
                    finding_id: misses[i].finding_id.clone(),
                    topic: misses[i].topic.clone(),
                    content: misses[i].content.clone(),
                    run_id: misses[i].run_id.clone(),
                })
                .collect();
            let size = members
                .iter()
                .map(|m| m.finding_id.as_str())
                .collect::<HashSet<_>>()
                .len();
            let runs = members
                .iter()
                .map(|m| m.run_id.as_str())
                .collect::<HashSet<_>>()
                .len();
            (size >= cfg.min_cluster_size && runs >= cfg.min_distinct_runs).then_some(
                ExpansionCandidate {
                    size,
                    runs,
                    members,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use mif_ontology::{CalibrationConfig, ConfidenceTier, ExpansionConfig};

    use super::{STATUS_PENDING, SuggestionEntry, expansion_candidates, upsert_suggestions};
    use crate::index::Miss;
    use crate::suggest::TypeSuggestion;

    fn entry(finding_id: &str, run_id: &str, status: &str) -> SuggestionEntry {
        SuggestionEntry {
            finding_id: finding_id.to_string(),
            file: None,
            basis: "untyped".to_string(),
            run_id: run_id.to_string(),
            candidates: vec![TypeSuggestion {
                entity_type: "control".to_string(),
                ontology_id: "sec".to_string(),
                score: 0.7,
                tier: ConfidenceTier::FlagForReview,
                margin: None,
                calibrated: false,
                negative_demoted: false,
            }],
            status: status.to_string(),
        }
    }

    fn miss(finding_id: &str, run_id: &str, vector: Vec<f32>) -> Miss {
        Miss {
            finding_id: finding_id.to_string(),
            topic: "sec".to_string(),
            content: format!("content of {finding_id}"),
            vector,
            run_id: run_id.to_string(),
            model: "test-model".to_string(),
        }
    }

    #[test]
    fn upsert_creates_refreshes_pending_and_preserves_verdicts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("suggestions/sec.json");

        // First run: two pending entries.
        upsert_suggestions(
            &path,
            "sec",
            vec![
                entry("f-1", "run-1", STATUS_PENDING),
                entry("f-2", "run-1", STATUS_PENDING),
            ],
        )
        .unwrap();

        // A human confirms f-1.
        let contents = std::fs::read_to_string(&path).unwrap();
        let confirmed = contents.replacen("\"pending\"", "\"confirmed\"", 1);
        std::fs::write(&path, confirmed).unwrap();

        // Second run re-suggests both plus a new finding.
        let queue = upsert_suggestions(
            &path,
            "sec",
            vec![
                entry("f-1", "run-2", STATUS_PENDING),
                entry("f-2", "run-2", STATUS_PENDING),
                entry("f-3", "run-2", STATUS_PENDING),
            ],
        )
        .unwrap();

        assert_eq!(queue.entries.len(), 3);
        // f-1's human verdict survives (run_id still run-1).
        let f1 = queue
            .entries
            .iter()
            .find(|e| e.finding_id == "f-1")
            .unwrap();
        assert_eq!(f1.status, "confirmed");
        assert_eq!(f1.run_id, "run-1");
        // f-2 was pending and got refreshed.
        let f2 = queue
            .entries
            .iter()
            .find(|e| e.finding_id == "f-2")
            .unwrap();
        assert_eq!(f2.run_id, "run-2");
        // Entries are sorted by finding_id.
        let ids: Vec<&str> = queue
            .entries
            .iter()
            .map(|e| e.finding_id.as_str())
            .collect();
        assert_eq!(ids, ["f-1", "f-2", "f-3"]);
    }

    #[test]
    fn a_queue_recorded_for_another_topic_is_rejected_not_absorbed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sec.json");
        upsert_suggestions(&path, "sec", vec![entry("f-1", "run-1", STATUS_PENDING)]).unwrap();

        let error = upsert_suggestions(&path, "edu", vec![]).unwrap_err();
        assert!(matches!(
            error,
            crate::MifRhError::QueueTopicMismatch { .. }
        ));
    }

    #[test]
    fn a_corrupt_queue_file_is_an_explicit_error_never_a_silent_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sec.json");
        std::fs::write(&path, "{ not json").unwrap();

        let error = upsert_suggestions(&path, "sec", vec![]).unwrap_err();
        assert!(matches!(error, crate::MifRhError::Json { .. }));
    }

    #[test]
    fn expansion_candidates_require_size_and_distinct_run_gates() {
        let cfg = ExpansionConfig {
            cluster_similarity: 0.95,
            min_cluster_size: 2,
            min_distinct_runs: 2,
        };
        // f-1 and f-2 are near-identical misses, but both from run-1: fails
        // the distinct-runs gate.
        let one_run = [
            miss("f-1", "run-1", vec![1.0, 0.0]),
            miss("f-2", "run-1", vec![1.0, 0.0]),
        ];
        assert!(expansion_candidates(&one_run, &cfg).is_empty());

        // The same cluster spanning two runs surfaces.
        let two_runs = [
            miss("f-1", "run-1", vec![1.0, 0.0]),
            miss("f-2", "run-2", vec![1.0, 0.0]),
            miss("f-3", "run-2", vec![0.0, 1.0]), // dissimilar outlier
        ];
        let candidates = expansion_candidates(&two_runs, &cfg);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].size, 2);
        assert_eq!(candidates[0].runs, 2);
    }

    #[test]
    fn the_same_finding_recurring_across_runs_does_not_inflate_cluster_size() {
        let cfg = ExpansionConfig {
            cluster_similarity: 0.95,
            min_cluster_size: 2,
            min_distinct_runs: 2,
        };
        // One finding missing twice is recurrence of ONE concept-instance,
        // not a two-finding cluster.
        let same_finding = [
            miss("f-1", "run-1", vec![1.0, 0.0]),
            miss("f-1", "run-2", vec![1.0, 0.0]),
        ];
        assert!(expansion_candidates(&same_finding, &cfg).is_empty());
    }

    #[test]
    fn calibration_expansion_defaults_flow_through() {
        // The default knobs come from the calibration artifact's expansion
        // block — spot-check the defaults documented in mif-ontology.
        let cfg = CalibrationConfig::default().expansion;
        assert!((cfg.cluster_similarity - 0.80).abs() < f32::EPSILON);
        assert_eq!(cfg.min_cluster_size, 3);
        assert_eq!(cfg.min_distinct_runs, 2);
    }
}
