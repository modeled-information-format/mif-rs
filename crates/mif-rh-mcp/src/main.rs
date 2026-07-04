//! MCP server for [`mif_rh`], the compiled ontology resolution/review
//! engine for research-harness-template (rht) corpora.
//!
//! Exposes `search`, `suggest_type`, `find_similar`, and `corpus_stats` as
//! read-only MCP tools over the index `mif-rh-cli review --build-index`
//! builds. This
//! server has **no filesystem write access to `reports/`** — `suggest_type`
//! returns a ranked hypothesis for a human or agent to confirm via rht's
//! own `/ontology-review --enrich` step, never an auto-stamp. Every tool
//! failure renders as a compact RFC 9457 `application/problem+json`
//! envelope, matching `mif-mcp`'s own convention (an MCP client is
//! inherently a machine consumer).

use std::path::{Path, PathBuf};

use mif_problem::{ProblemMeta, ToProblem};
use mif_rh::index::FindingIndex;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Serialize;

const DEFAULT_INDEX_PATH: &str = "reports/_meta/search-index.sqlite";
const DEFAULT_REPORTS_DIR: &str = "reports";
const DEFAULT_CATALOG: &str = ".claude/enabled-packs.json";
const DEFAULT_CONFIG: &str = "harness.config.json";
const DEFAULT_LIMIT: usize = 10;

/// Errors reported by the `mif-rh-mcp` binary itself.
#[derive(Debug, thiserror::Error)]
enum McpError {
    /// A tool that reads the search index was called before
    /// `mif-rh-cli review` has ever built one.
    #[error("index not built at {path} — run `mif-rh-cli review` first")]
    IndexNotBuilt {
        /// The index path that does not exist.
        path: String,
    },
    /// Loading the ontology corpus, resolving, or reading the index failed.
    #[error(transparent)]
    MifRh(#[from] mif_rh::MifRhError),
}

impl McpError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::IndexNotBuilt { .. } => ProblemMeta {
                slug: "index-not-built",
                version: "v1",
                title: "Search index has not been built yet",
                status: 404,
                exit_code: 1,
            },
            Self::MifRh(_) => ProblemMeta {
                slug: "delegated",
                version: "v1",
                title: "Delegated error",
                status: 500,
                exit_code: 1,
            },
        }
    }
}

impl ToProblem for McpError {
    fn to_problem(&self) -> mif_problem::ProblemDetails {
        match self {
            Self::MifRh(inner) => inner.to_problem(),
            Self::IndexNotBuilt { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string())
                .with_suggested_fix(mif_problem::SuggestedFix::new(
                    "Run `mif-rh-cli review --build-index` to build the search index, then retry.",
                    mif_problem::Applicability::MachineApplicable,
                ))
                .with_code_action(mif_problem::CodeAction::new(
                    "Build the search index",
                    "quickfix",
                    mif_problem::Applicability::MachineApplicable,
                )),
        }
    }
}

fn resolve_path(given: Option<&Path>, default: &str) -> PathBuf {
    given.map_or_else(|| PathBuf::from(default), Path::to_path_buf)
}

fn open_index(index_path: &Path) -> Result<FindingIndex, McpError> {
    if !index_path.exists() {
        return Err(McpError::IndexNotBuilt {
            path: index_path.display().to_string(),
        });
    }
    Ok(FindingIndex::open(index_path)?)
}

/// Parameters for the `search` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// The full-text query.
    query: String,
    /// Maximum number of ranked results to return. Defaults to 10.
    limit: Option<usize>,
    /// Path to the search index. Defaults to
    /// `reports/_meta/search-index.sqlite`.
    index_path: Option<PathBuf>,
}

/// One full-text search result.
#[derive(Debug, Serialize)]
struct SearchHit {
    finding_id: String,
    topic: String,
    snippet: String,
    score: f64,
}

/// Parameters for the `suggest_type` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SuggestTypeParams {
    /// The text to classify.
    text: String,
    /// The topic whose bound ontologies supply the candidate entity types.
    topic: String,
    /// Path to the ontology catalog. Defaults to
    /// `.claude/enabled-packs.json`.
    catalog: Option<PathBuf>,
    /// Path to the harness config. Defaults to `harness.config.json`.
    config: Option<PathBuf>,
    /// Base directory ontology catalog `source` paths resolve against.
    /// Defaults to the current directory.
    root: Option<PathBuf>,
    /// Maximum number of ranked candidates to return. Defaults to 10.
    limit: Option<usize>,
}

/// One ranked entity-type hypothesis. A hypothesis, never a stamp — this
/// tool has no write access to `reports/`.
#[derive(Debug, Serialize)]
struct EntityTypeSuggestion {
    entity_type: String,
    ontology_id: String,
    score: f32,
}

/// Parameters for the `find_similar` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct FindSimilarParams {
    /// The text to find similar findings for.
    text: String,
    /// Maximum number of ranked results to return. Defaults to 10.
    limit: Option<usize>,
    /// A finding id to exclude from the results (e.g. the finding whose own
    /// content is the query).
    exclude_finding_id: Option<String>,
    /// Path to the search index. Defaults to
    /// `reports/_meta/search-index.sqlite`.
    index_path: Option<PathBuf>,
}

/// One similarity match, ranked by cosine similarity, across every topic
/// in the corpus.
#[derive(Debug, Serialize)]
struct SimilarityHit {
    finding_id: String,
    topic: String,
    score: f32,
}

/// Parameters for the `corpus_stats` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CorpusStatsParams {
    /// Root `reports/` directory. Defaults to `reports`.
    reports_dir: Option<PathBuf>,
}

/// The result of a `corpus_stats` query: the same aggregate
/// `ontology-review.sh`'s own summary line reports, read from each topic's
/// already-written `ontology-map.json` rather than a fresh classification
/// pass (this server never resolves or writes).
#[derive(Debug, Serialize)]
struct CorpusStatsResult {
    topics: u64,
    findings: u64,
    stamped: u64,
    discovery: u64,
    untyped: u64,
    invalid: u64,
}

/// Reads every `reports/<topic>/ontology-map.json` under `reports_dir` and
/// aggregates the same coverage buckets `review()` computes, without
/// re-resolving anything.
fn corpus_stats_inner(reports_dir: &Path) -> CorpusStatsResult {
    use mif_rh::Basis;

    let mut topics = 0_u64;
    let mut findings = 0_u64;
    let mut stamped = 0_u64;
    let mut discovery = 0_u64;
    let mut untyped = 0_u64;
    let mut invalid = 0_u64;

    let Ok(entries) = std::fs::read_dir(reports_dir) else {
        return CorpusStatsResult {
            topics: 0,
            findings: 0,
            stamped: 0,
            discovery: 0,
            untyped: 0,
            invalid: 0,
        };
    };

    for entry in entries.filter_map(Result::ok) {
        let map_path = entry.path().join("ontology-map.json");
        let Ok(contents) = std::fs::read_to_string(&map_path) else {
            continue;
        };
        let Ok(records) = serde_json::from_str::<Vec<mif_rh::MapRecord>>(&contents) else {
            continue;
        };
        topics += 1;
        for record in &records {
            findings += 1;
            match record.basis {
                _ if !record.valid => invalid += 1,
                Basis::Declared | Basis::Resolved => stamped += 1,
                Basis::Discovery => discovery += 1,
                Basis::Untyped => untyped += 1,
                Basis::Unresolved | Basis::Ambiguous => invalid += 1,
            }
        }
    }

    CorpusStatsResult {
        topics,
        findings,
        stamped,
        discovery,
        untyped,
        invalid,
    }
}

fn search_inner(query: &str, limit: usize, index_path: &Path) -> Result<Vec<SearchHit>, McpError> {
    let index = open_index(index_path)?;
    let matches = index.search(query, limit)?;
    Ok(matches
        .into_iter()
        .map(|m| SearchHit {
            finding_id: m.finding_id,
            topic: m.topic,
            snippet: m.snippet,
            score: m.score,
        })
        .collect())
}

fn find_similar_inner(
    text: &str,
    limit: usize,
    exclude_finding_id: Option<&str>,
    index_path: &Path,
) -> Result<Vec<SimilarityHit>, McpError> {
    let index = open_index(index_path)?;
    let embedder = mif_embed::Embedder::load().map_err(mif_rh::MifRhError::from)?;
    let vector = embedder.embed(text).map_err(mif_rh::MifRhError::from)?;
    let matches = index.find_similar(&vector, limit, exclude_finding_id)?;
    Ok(matches
        .into_iter()
        .map(|m| SimilarityHit {
            finding_id: m.finding_id,
            topic: m.topic,
            score: m.score,
        })
        .collect())
}

/// Embeds `text` and every candidate entity-type description for `topic`'s
/// currently allowed ontologies (small candidate set — a handful of entity
/// types per topic — so this is computed live, with no persistent index),
/// ranking by cosine similarity. Never writes anywhere.
fn suggest_type_inner(
    text: &str,
    topic: &str,
    catalog_path: &Path,
    config_path: &Path,
    root: &Path,
    limit: usize,
) -> Result<Vec<EntityTypeSuggestion>, McpError> {
    let catalog = mif_rh::Catalog::load(catalog_path)?;
    let config = mif_rh::HarnessConfig::load(config_path)?;
    let ontology_packs = mif_rh::ontology_pack::load_packs_via_catalog(&catalog, root)?;

    let ctx = mif_rh::ResolveContext {
        topic,
        catalog: &catalog,
        config: &config,
        ontology_packs: &ontology_packs,
    };
    let allowed = mif_rh::build_allowed(&ctx)?;

    let embedder = mif_embed::Embedder::load().map_err(mif_rh::MifRhError::from)?;
    let query_vector = embedder.embed(text).map_err(mif_rh::MifRhError::from)?;

    let mut suggestions = Vec::new();
    for pack in &allowed {
        for entity_type in &pack.entity_types {
            let Some(description) = &entity_type.description else {
                continue;
            };
            let candidate_vector = embedder
                .embed(description)
                .map_err(mif_rh::MifRhError::from)?;
            let score = mif_rh::index::cosine_similarity(&query_vector, &candidate_vector);
            suggestions.push(EntityTypeSuggestion {
                entity_type: entity_type.name.clone(),
                ontology_id: pack.id.clone(),
                score,
            });
        }
    }

    suggestions.sort_by(|a, b| b.score.total_cmp(&a.score));
    suggestions.truncate(limit);
    Ok(suggestions)
}

#[derive(Clone)]
struct MifRh;

// rmcp's #[tool] macro requires an instance method (&self receiver) for its
// dispatch mechanism, even though these handlers are stateless.
#[allow(clippy::unused_self)]
#[tool_router]
impl MifRh {
    #[tool(description = "Full-text search over the mif-rh finding index")]
    fn search(
        &self,
        Parameters(SearchParams {
            query,
            limit,
            index_path,
        }): Parameters<SearchParams>,
    ) -> String {
        let index_path = resolve_path(index_path.as_deref(), DEFAULT_INDEX_PATH);
        match search_inner(&query, limit.unwrap_or(DEFAULT_LIMIT), &index_path) {
            Ok(hits) => serde_json::to_string(&hits).unwrap_or_else(|_| "[]".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(
        description = "Suggest candidate entity types for a piece of text, ranked by embedding \
                        similarity to a topic's bound ontologies' entity-type descriptions. A \
                        hypothesis only — never writes to reports/"
    )]
    fn suggest_type(
        &self,
        Parameters(SuggestTypeParams {
            text,
            topic,
            catalog,
            config,
            root,
            limit,
        }): Parameters<SuggestTypeParams>,
    ) -> String {
        let catalog_path = resolve_path(catalog.as_deref(), DEFAULT_CATALOG);
        let config_path = resolve_path(config.as_deref(), DEFAULT_CONFIG);
        let root = resolve_path(root.as_deref(), ".");
        match suggest_type_inner(
            &text,
            &topic,
            &catalog_path,
            &config_path,
            &root,
            limit.unwrap_or(DEFAULT_LIMIT),
        ) {
            Ok(suggestions) => {
                serde_json::to_string(&suggestions).unwrap_or_else(|_| "[]".to_string())
            },
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(description = "Find findings similar to a piece of text, across every topic")]
    fn find_similar(
        &self,
        Parameters(FindSimilarParams {
            text,
            limit,
            exclude_finding_id,
            index_path,
        }): Parameters<FindSimilarParams>,
    ) -> String {
        let index_path = resolve_path(index_path.as_deref(), DEFAULT_INDEX_PATH);
        match find_similar_inner(
            &text,
            limit.unwrap_or(DEFAULT_LIMIT),
            exclude_finding_id.as_deref(),
            &index_path,
        ) {
            Ok(hits) => serde_json::to_string(&hits).unwrap_or_else(|_| "[]".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(description = "Aggregate ontology classification coverage across every reviewed topic")]
    fn corpus_stats(
        &self,
        Parameters(CorpusStatsParams { reports_dir }): Parameters<CorpusStatsParams>,
    ) -> String {
        let reports_dir = resolve_path(reports_dir.as_deref(), DEFAULT_REPORTS_DIR);
        let stats = corpus_stats_inner(&reports_dir);
        serde_json::to_string(&stats).unwrap_or_else(|_| "{}".to_string())
    }
}

#[tool_handler(
    name = "mif-rh-mcp",
    instructions = "Search, suggest entity types for, and find similar research-harness-template \
                    findings. Read-only: never writes to reports/"
)]
impl ServerHandler for MifRh {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = MifRh.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use mif_rh::index::IndexedFinding;

    use super::{corpus_stats_inner, find_similar_inner, search_inner};

    #[test]
    fn search_reports_index_not_built_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index.sqlite");
        let error = search_inner("anything", 10, &index_path).unwrap_err();
        assert!(matches!(error, super::McpError::IndexNotBuilt { .. }));
    }

    #[test]
    fn search_finds_matching_content_once_built() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index.sqlite");
        let mut index = mif_rh::index::FindingIndex::open(&index_path).unwrap();
        index
            .rebuild(&[IndexedFinding {
                finding_id: "f-1".to_string(),
                topic: "edu".to_string(),
                content: "a great textbook about algebra".to_string(),
                vector: vec![1.0, 0.0],
            }])
            .unwrap();

        let hits = search_inner("textbook", 10, &index_path).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].finding_id, "f-1");
    }

    #[test]
    fn find_similar_reports_index_not_built_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index.sqlite");
        let error = find_similar_inner("anything", 10, None, &index_path).unwrap_err();
        assert!(matches!(error, super::McpError::IndexNotBuilt { .. }));
    }

    #[test]
    fn corpus_stats_aggregates_across_topics_from_ontology_map_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("edu")).unwrap();
        fs::write(
            dir.path().join("edu/ontology-map.json"),
            r#"[
                {"finding_id":"f-1","entity_type":"title","resolved_ontology":"edu-fixture@0.1.0","basis":"resolved","valid":true},
                {"finding_id":"f-2","entity_type":null,"resolved_ontology":null,"basis":"untyped","valid":true},
                {"finding_id":"f-3","entity_type":"title","resolved_ontology":null,"basis":"unresolved","valid":false}
            ]"#,
        )
        .unwrap();

        let stats = corpus_stats_inner(dir.path());
        assert_eq!(stats.topics, 1);
        assert_eq!(stats.findings, 3);
        assert_eq!(stats.stamped, 1);
        assert_eq!(stats.untyped, 1);
        assert_eq!(stats.invalid, 1);
    }

    #[test]
    fn corpus_stats_on_a_missing_reports_dir_returns_zeroes_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let stats = corpus_stats_inner(&dir.path().join("nonexistent"));
        assert_eq!(stats.topics, 0);
        assert_eq!(stats.findings, 0);
    }
}
