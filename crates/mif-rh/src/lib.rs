//! Compiled ontology resolution/review engine for
//! [research-harness-template](https://github.com/modeled-information-format/research-harness-template)
//! (rht) corpora, in the [MIF (Modeled Information Format)](https://mif-spec.dev)
//! ecosystem.
//!
//! Reimplements the observable behavior of rht's `scripts/resolve-ontology.sh`
//! ([`resolve::resolve_finding`]) and `scripts/ontology-review.sh`
//! ([`review::review`]) — classifying findings against topic-bound domain
//! ontologies, validating each finding's `entity` payload, and aggregating
//! per-topic coverage — without any `yq`/`jq`/`ajv` subprocess dependency.
//!
//! # Determinism boundary
//!
//! [`resolve::resolve_finding`] and [`review::review`] are entirely
//! deterministic and rule-based: exact string matching against an
//! ontology's declared entity types, regex matching against discovery
//! patterns, and JSON Schema validation. **No embeddings are used in this
//! classification path** — it must stay deterministic both for byte-for-byte
//! parity with rht's bash output and because rht's own fail-closed gate
//! (ADR-0011) consumes `ontology-map.json`'s `basis`/`valid` fields
//! directly to decide whether a finding can ship.
//!
//! Embeddings and cosine similarity are used only by the hypothesis layer:
//! [`index::FindingIndex`] (full-text and embedding search over a corpus)
//! and [`suggest::suggest_type`] (tier-annotated entity-type hypotheses per
//! MIF ADR-020's confidence policy, via [`mif_ontology::confidence`]) —
//! both consumed by the `mif-rh-mcp` server's
//! `search`/`suggest_type`/`find_similar` tools and by `mif-rh-cli`'s
//! `suggest-type` subcommand. That layer is read-only and
//! never-authoritative: it never writes to `ontology-map.json`, and
//! `resolve`/`review` never call into it.

pub mod author;
pub mod calibrate;
pub mod catalog;
pub mod config;
mod error;
pub mod finding;
pub mod harness_assert_graph;
pub mod harness_citation_integrity;
pub mod harness_concordance;
pub mod harness_corpus;
pub mod harness_falsify;
pub mod harness_graph;
pub mod harness_import;
pub mod harness_index;
pub mod harness_markdown;
pub mod harness_membership;
pub mod harness_project;
pub mod harness_reconcile;
pub mod harness_relationship_targets;
pub mod harness_release;
pub mod harness_render;
pub mod harness_shippable_typing;
pub mod harness_synthesize;
pub mod harness_toggle;
pub mod harness_topic_metadata;
pub mod harness_wrap;
pub mod index;
pub mod lock;
pub mod ontology_pack;
pub mod queue;
pub mod resolve;
pub mod review;
pub mod suggest;
pub mod vendor;

pub use author::{DraftReport, draft_from_clusters, draft_from_topic};
pub use calibrate::{
    CONFUSION_REPRESENTATIVES, CalibrateOptions, CalibrationSample, ConfusionPair, ConfusionReport,
    collect_topic_samples, confusions, packs_carry_negatives, subsample, sweep,
};
pub use catalog::Catalog;
pub use config::HarnessConfig;
pub use error::MifRhError;
pub use finding::Finding;
pub use harness_assert_graph::{CheckResult, GraphAssertion, assert_graph_mif_file};
pub use harness_citation_integrity::{CitationIntegrityReport, check_citation_integrity};
pub use harness_concordance::build_concordance;
pub use harness_corpus::{CorpusSynthesis, synthesize_corpus};
pub use harness_falsify::{FalsifyResult, falsify};
pub use harness_graph::build_graph;
pub use harness_import::{ImportReport, import_corpus};
pub use harness_index::build_index;
pub use harness_membership::{MembershipReport, resolve_membership};
pub use harness_project::project_report;
pub use harness_reconcile::{ReconcileReport, reconcile_session, sort_object_keys};
pub use harness_relationship_targets::{
    Orphan, RelationshipTargetsReport, check_relationship_targets,
};
pub use harness_release::{
    BumpOptions, BumpReport, VersionGateFailure, VersionGateReport, bump_version,
    check_version_bump, goal_version_id,
};
pub use harness_render::{RenderInputs, render_artifact};
pub use harness_shippable_typing::{ShippableTypingReport, check_shippable_typing};
pub use harness_synthesize::synthesize_artifact;
pub use harness_toggle::{SITE_PLUGINS, pack_toggle, site_toggle_plugin, site_toggle_primary};
pub use harness_topic_metadata::{TopicMetadata, topic_metadata};
pub use harness_wrap::{WrapSourceInputs, read_source_content, wrap_source};
pub use index::{FindingIndex, IndexStats, IndexedFinding, Miss, SearchMatch, SimilarFinding};
pub use lock::ReviewLock;
pub use ontology_pack::{EntityType, OntologyPack};
pub use queue::{
    ClusterMember, ExpansionCandidate, SuggestionEntry, SuggestionQueue, expansion_candidates,
    upsert_suggestions,
};
pub use resolve::{Basis, MapRecord, ResolveContext, build_allowed, resolve_finding};
pub use review::{
    FollowupBacklog, FollowupEntry, ReviewOptions, ReviewReport, TopicSummary, review,
    write_followup,
};
pub use suggest::{TypeSuggestion, suggest_type};
pub use vendor::{
    CatalogSyncReport, DriftEntry, FetchReport, LockCheckReport, LockEntry, LockFile,
    RegistrySyncReport, VendoredOntology, fetch, lock_check, resolve_source, sync_catalog,
    sync_registry,
};

/// Rebuilds the search index for `topic_ids`, embedding every finding's
/// discovery text (or its entity's `name`, for typed findings with no
/// standalone content field) via [`mif_embed::Embedder`].
///
/// A separate step from [`review::review`] — index rebuilding always
/// re-embeds every finding, which is far more expensive than the
/// deterministic classification pass, so callers that only need
/// classification (e.g. a fail-closed CI gate) are not forced to pay for
/// it.
///
/// # Errors
///
/// Returns [`MifRhError`] if a topic's findings directory cannot be read,
/// a finding fails to parse, the embedding model cannot be loaded or run,
/// or the index cannot be rebuilt.
pub fn build_search_index(
    reports_dir: &std::path::Path,
    topic_ids: &[String],
    index: &mut FindingIndex,
) -> Result<(), MifRhError> {
    let embedder = mif_embed::Embedder::load()?;
    let mut indexed = Vec::new();

    for topic in topic_ids {
        let findings_dir = reports_dir.join(topic).join("findings");
        if !findings_dir.is_dir() {
            continue;
        }
        for file in review::list_finding_files(&findings_dir)? {
            let finding = Finding::load(&file)?;
            let text = index_text(&finding);
            let vector = embedder.embed(&text)?;
            indexed.push(IndexedFinding {
                finding_id: finding.id,
                topic: topic.clone(),
                content: text,
                vector,
            });
        }
    }

    index.rebuild(&indexed)
}

/// Serializes `value` to pretty JSON and atomically writes it to `path`.
///
/// Writes to a `.tmp` sibling, then renames over the destination — so a
/// reader never observes a partially-written file. Shared by
/// [`review::write_followup`], `review`'s own internal `ontology-map.json`
/// writer, and `mif-rh-cli`'s `ontology-map.json` upsert, which all follow
/// the exact same write-then-rename shape.
///
/// # Errors
///
/// Returns [`MifRhError::JsonSerialize`] if `value` cannot be serialized,
/// or [`MifRhError::Io`] if the temporary file cannot be written or
/// renamed into place.
pub fn write_json_atomic<T: serde::Serialize>(
    path: &std::path::Path,
    value: &T,
) -> Result<(), MifRhError> {
    let json = serde_json::to_string_pretty(value).map_err(|source| MifRhError::JsonSerialize {
        path: path.display().to_string(),
        source,
    })?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json).map_err(|source| MifRhError::Io {
        path: tmp_path.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp_path, path).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// The text embedded and indexed for one finding: its discovery text
/// (typically `content`) if any, otherwise its entity's `name`, if any.
///
/// Public so binary frontends can derive the same query text from a
/// finding file (e.g. `mif-rh-cli suggest-type --finding <path>`) that the
/// index itself embeds.
#[must_use]
pub fn index_text(finding: &Finding) -> String {
    let discovery_text = finding.discovery_text();
    if !discovery_text.is_empty() {
        return discovery_text;
    }
    finding
        .entity
        .as_ref()
        .and_then(|entity| entity.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}
