//! MCP server for the MIF (Modeled Information Format) ecosystem.
//!
//! Exposes six operations as MCP tools: `validate_mif_document`,
//! `resolve_ontology_reference`, `ingest_mif_document`,
//! `search_documents`, `find_similar_documents`, and `corpus_stats`. Each is
//! a thin wrapper calling the identical `mif-schema`/`mif-ontology`/
//! `mif-frontmatter`/`mif-embed`/`mif-store` functions `mif-cli` calls —
//! kept deliberately in lockstep rather than diverging.
//!
//! An MCP client is inherently a machine consumer (there is no terminal to
//! detect), so every failure renders as a compact RFC 9457
//! `application/problem+json` envelope via [`mif_problem`] rather than plain
//! text — see [`McpError::to_problem`].

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use mif_problem::{ProblemMeta, ToProblem};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};

/// Default path for the vector store database, relative to the current
/// working directory, when `db_path` is not given.
const DEFAULT_DB_PATH: &str = ".mif/vectors.db";

/// Parameters for the `validate_mif_document` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ValidateParams {
    /// Path to the MIF document (JSON-LD projection) to validate.
    file: PathBuf,
}

/// Parameters for the `resolve_ontology_reference` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ResolveParams {
    /// The ontology ID to resolve.
    id: String,
    /// Directory containing ontology definition YAML files.
    ontologies_dir: PathBuf,
}

/// Parameters for the `ingest_mif_document` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct IngestParams {
    /// Path to the MIF document (markdown with frontmatter, or a JSON-LD
    /// projection) to ingest.
    file: PathBuf,
    /// Path to the `SQLite` vector store database. Defaults to
    /// `.mif/vectors.db`, created (along with its parent directory) if
    /// absent.
    db_path: Option<PathBuf>,
}

/// The result of successfully ingesting one MIF document.
#[derive(Debug, serde::Serialize)]
struct IngestReport {
    /// Always `"ok"` on success (lint and validate are the same schema
    /// validation step; see `mif-cli`'s `CLAUDE.md` for the rationale).
    lint: &'static str,
    /// Always `"ok"` on success.
    validate: &'static str,
    /// Always `"lossless"` on success.
    roundtrip: &'static str,
    /// Dimensionality of the stored embedding vector.
    embedding_dim: usize,
    /// Always `true` on success.
    stored: bool,
    /// The document ID the embedding was stored under.
    id: String,
    /// The vector store database path used.
    db: String,
}

/// Parameters for the `search_documents` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// The query text to embed and rank stored documents against.
    query: String,
    /// Path to the `SQLite` vector store database. Defaults to
    /// `.mif/vectors.db`.
    db_path: Option<PathBuf>,
    /// Maximum number of ranked results to return. Defaults to 10.
    limit: Option<usize>,
}

/// Parameters for the `find_similar_documents` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct FindSimilarParams {
    /// The id of an already-ingested document (as returned by
    /// `ingest_mif_document`).
    id: String,
    /// Path to the `SQLite` vector store database. Defaults to
    /// `.mif/vectors.db`.
    db_path: Option<PathBuf>,
    /// Maximum number of ranked results to return, excluding `id` itself.
    /// Defaults to 10.
    limit: Option<usize>,
}

/// Parameters for the `corpus_stats` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CorpusStatsParams {
    /// Path to the `SQLite` vector store database. Defaults to
    /// `.mif/vectors.db`.
    db_path: Option<PathBuf>,
}

/// The default result limit for `search_documents`/`find_similar_documents`
/// when the caller does not specify one.
const DEFAULT_LIMIT: usize = 10;

/// One ranked match returned by `search_documents`/`find_similar_documents`.
#[derive(Debug, serde::Serialize)]
struct MatchEntry {
    /// The matching document's id.
    id: String,
    /// Cosine similarity to the query, in `-1.0..=1.0`.
    score: f32,
}

/// The result of a `search_documents`/`find_similar_documents` query.
#[derive(Debug, serde::Serialize)]
struct SearchResult {
    /// Ranked matches, most similar first.
    matches: Vec<MatchEntry>,
}

/// The result of a `corpus_stats` query.
#[derive(Debug, serde::Serialize)]
struct CorpusStatsResult {
    /// Total number of embeddings stored.
    count: u64,
    /// The dimensionality of a stored embedding, if the store is non-empty.
    dim: Option<usize>,
    /// The vector store database path used.
    db: String,
}

/// Errors reported by the `mif-mcp` binary itself.
#[derive(Debug, thiserror::Error)]
enum McpError {
    /// Failed to read an input file.
    #[error("failed to read {path}: {source}")]
    Io {
        /// The path that failed to read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The input file was not valid JSON.
    #[error("failed to parse {path} as JSON: {source}")]
    Json {
        /// The path that failed to parse.
        path: String,
        /// The underlying parse error.
        #[source]
        source: serde_json::Error,
    },
    /// Schema validation failed.
    #[error(transparent)]
    Schema(#[from] mif_schema::MifSchemaError),
    /// Ontology loading or resolution failed.
    #[error(transparent)]
    Ontology(#[from] mif_ontology::OntologyError),
    /// Frontmatter parsing, projection, or round-trip verification failed.
    #[error(transparent)]
    Frontmatter(#[from] mif_frontmatter::FrontmatterError),
    /// Computing the document's embedding failed.
    #[error(transparent)]
    Embed(#[from] mif_embed::EmbedError),
    /// Storing the embedding vector failed.
    #[error(transparent)]
    Store(#[from] mif_store::StoreError),
    /// A `find_similar_documents` query named an id that has never been
    /// ingested.
    #[error("no document with id '{0}' has been ingested into this vector store")]
    DocumentNotFound(String),
}

impl McpError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::Io { .. } => ProblemMeta {
                slug: "io",
                version: "v1",
                title: "Failed to read an input file",
                status: 500,
                exit_code: 1,
            },
            Self::Json { .. } => ProblemMeta {
                slug: "invalid-json",
                version: "v1",
                title: "Input file is not valid JSON",
                status: 400,
                exit_code: 2,
            },
            Self::DocumentNotFound(_) => ProblemMeta {
                slug: "document-not-found",
                version: "v1",
                title: "No document with the given id has been ingested",
                status: 404,
                exit_code: 3,
            },
            // Schema/Ontology/Frontmatter/Embed/Store carry their own
            // `ProblemMeta` internally; see `to_problem` below, which
            // delegates to them directly.
            Self::Schema(_)
            | Self::Ontology(_)
            | Self::Frontmatter(_)
            | Self::Embed(_)
            | Self::Store(_) => ProblemMeta {
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
            Self::Schema(inner) => inner.to_problem(),
            Self::Ontology(inner) => inner.to_problem(),
            Self::Frontmatter(inner) => inner.to_problem(),
            Self::Embed(inner) => inner.to_problem(),
            Self::Store(inner) => inner.to_problem(),
            Self::Io { source, .. } => {
                let (status, fix, action) = mif_problem::classify_io_error(source);
                let mut problem = self
                    .meta()
                    .into_details(env!("CARGO_PKG_NAME"), self.to_string());
                problem.status = status;
                problem.with_suggested_fix(fix).with_code_action(action)
            },
            Self::DocumentNotFound(_) => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string())
                .with_suggested_fix(mif_problem::SuggestedFix::new(
                    "Ingest the document first with the `ingest_mif_document` tool, or check \
                     the id for a typo.",
                    mif_problem::Applicability::MaybeIncorrect,
                ))
                .with_code_action(mif_problem::CodeAction::new(
                    "Ingest the document before searching for similar ones",
                    "quickfix",
                    mif_problem::Applicability::MaybeIncorrect,
                )),
            Self::Json { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string()),
        }
    }
}

fn validate_mif_document_inner(file: &Path) -> Result<String, McpError> {
    let contents = std::fs::read_to_string(file).map_err(|source| McpError::Io {
        path: file.display().to_string(),
        source,
    })?;
    let instance: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| McpError::Json {
            path: file.display().to_string(),
            source,
        })?;
    mif_schema::validate_document(&instance)?;
    Ok(format!("{}: valid", file.display()))
}

fn resolve_ontology_reference_inner(id: &str, ontologies_dir: &Path) -> Result<String, McpError> {
    let corpus = mif_ontology::load_corpus_from_dir(ontologies_dir)?;
    let chain = mif_ontology::resolve_chain(id, &corpus)?;
    Ok(chain
        .iter()
        .map(|ontology| format!("{} ({})", ontology.id, ontology.version))
        .collect::<Vec<_>>()
        .join(" -> "))
}

/// Projects `contents` to a JSON-LD document and proves the markdown <->
/// JSON-LD round trip is lossless, dispatching on whether `contents` is
/// markdown-with-frontmatter (starts with `---`) or already JSON-LD.
fn project_to_jsonld(contents: &str) -> Result<serde_json::Value, McpError> {
    if contents.trim_start().starts_with("---") {
        mif_frontmatter::roundtrip_lossless(contents)?;
        let (frontmatter, body) = mif_frontmatter::parse_markdown(contents)?;
        Ok(mif_frontmatter::md_to_jsonld(&frontmatter, &body)?)
    } else {
        let jsonld: serde_json::Value =
            serde_json::from_str(contents).map_err(|source| McpError::Json {
                path: "<ingest input>".to_string(),
                source,
            })?;
        let (frontmatter, body) =
            mif_frontmatter::jsonld_to_md(&jsonld, mif_frontmatter::FrontmatterShape::V1Canonical)?;
        let derived_md = mif_frontmatter::serialize_markdown(&frontmatter, &body)?;
        mif_frontmatter::roundtrip_lossless(&derived_md)?;
        Ok(jsonld)
    }
}

/// A stable, non-cryptographic hash of `contents`, used only to detect
/// whether a document's content changed since it was last ingested.
fn content_hash(contents: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Runs the full ingestion pipeline for one MIF document: validate (lint),
/// prove a lossless round trip, compute an embedding, and store it.
fn ingest_mif_document_inner(
    file: &Path,
    db_path: Option<&Path>,
) -> Result<IngestReport, McpError> {
    let contents = std::fs::read_to_string(file).map_err(|source| McpError::Io {
        path: file.display().to_string(),
        source,
    })?;

    let jsonld = project_to_jsonld(&contents)?;
    mif_schema::validate_document(&jsonld)?;

    let id = jsonld
        .get("@id")
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| file.display().to_string(), ToString::to_string);
    let content_text = jsonld
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&contents);

    let embedder = mif_embed::Embedder::load()?;
    let vector = embedder.embed(content_text)?;

    let db_path = db_path.map_or_else(|| PathBuf::from(DEFAULT_DB_PATH), Path::to_path_buf);
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| McpError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let store = mif_store::VectorStore::open(&db_path)?;
    let hash = content_hash(&contents);
    let updated_at = chrono::Utc::now().to_rfc3339();
    store.upsert(&id, &vector, &hash, &updated_at)?;

    Ok(IngestReport {
        lint: "ok",
        validate: "ok",
        roundtrip: "lossless",
        embedding_dim: vector.len(),
        stored: true,
        id,
        db: db_path.display().to_string(),
    })
}

/// Resolves an optional `db_path` to the effective vector store path.
fn resolve_db_path(db_path: Option<&Path>) -> PathBuf {
    db_path.map_or_else(|| PathBuf::from(DEFAULT_DB_PATH), Path::to_path_buf)
}

/// Converts `mif-store`'s similarity matches into this tool's result shape.
fn to_search_result(matches: Vec<mif_store::SimilarityMatch>) -> SearchResult {
    SearchResult {
        matches: matches
            .into_iter()
            .map(|m| MatchEntry {
                id: m.id,
                score: m.score,
            })
            .collect(),
    }
}

/// Embeds `query` and ranks previously ingested documents by cosine
/// similarity to it.
fn search_documents_inner(
    query: &str,
    db_path: Option<&Path>,
    limit: usize,
) -> Result<SearchResult, McpError> {
    let embedder = mif_embed::Embedder::load()?;
    let vector = embedder.embed(query)?;

    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let matches = store.top_k_similar(&vector, limit)?;

    Ok(to_search_result(matches))
}

/// Finds documents similar to an already-ingested one, identified by `id`.
fn find_similar_documents_inner(
    id: &str,
    db_path: Option<&Path>,
    limit: usize,
) -> Result<SearchResult, McpError> {
    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let anchor = store
        .get(id)?
        .ok_or_else(|| McpError::DocumentNotFound(id.to_string()))?;

    // Request one extra match so excluding the anchor document itself still
    // leaves up to `limit` genuinely-similar results.
    let matches: Vec<_> = store
        .top_k_similar(&anchor.vector, limit + 1)?
        .into_iter()
        .filter(|m| m.id != id)
        .take(limit)
        .collect();

    Ok(to_search_result(matches))
}

/// Summarizes the vector store's contents.
fn corpus_stats_inner(db_path: Option<&Path>) -> Result<CorpusStatsResult, McpError> {
    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let stats = store.stats()?;

    Ok(CorpusStatsResult {
        count: stats.count,
        dim: stats.dim,
        db: db_path.display().to_string(),
    })
}

#[derive(Clone)]
struct Mif;

// rmcp's #[tool] macro requires an instance method (&self receiver) for its
// dispatch mechanism, even though these handlers are stateless.
#[allow(clippy::unused_self)]
#[tool_router]
impl Mif {
    #[tool(description = "Validate a MIF document against the canonical MIF JSON Schema")]
    fn validate_mif_document(
        &self,
        Parameters(ValidateParams { file }): Parameters<ValidateParams>,
    ) -> String {
        validate_mif_document_inner(&file).unwrap_or_else(|error| error.to_problem().to_json())
    }

    #[tool(description = "Resolve an ontology's three-tier extends chain")]
    fn resolve_ontology_reference(
        &self,
        Parameters(ResolveParams { id, ontologies_dir }): Parameters<ResolveParams>,
    ) -> String {
        resolve_ontology_reference_inner(&id, &ontologies_dir)
            .unwrap_or_else(|error| error.to_problem().to_json())
    }

    #[tool(
        description = "Lint, validate, prove a lossless round trip, compute an embedding, and \
                        store the embedding vector for one MIF document"
    )]
    fn ingest_mif_document(
        &self,
        Parameters(IngestParams { file, db_path }): Parameters<IngestParams>,
    ) -> String {
        match ingest_mif_document_inner(&file, db_path.as_deref()) {
            Ok(report) => serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(description = "Free-text semantic search over previously ingested documents")]
    fn search_documents(
        &self,
        Parameters(SearchParams {
            query,
            db_path,
            limit,
        }): Parameters<SearchParams>,
    ) -> String {
        match search_documents_inner(&query, db_path.as_deref(), limit.unwrap_or(DEFAULT_LIMIT)) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(description = "Find previously ingested documents similar to an already-ingested one")]
    fn find_similar_documents(
        &self,
        Parameters(FindSimilarParams { id, db_path, limit }): Parameters<FindSimilarParams>,
    ) -> String {
        match find_similar_documents_inner(&id, db_path.as_deref(), limit.unwrap_or(DEFAULT_LIMIT))
        {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(description = "Summary statistics over the vector store")]
    fn corpus_stats(
        &self,
        Parameters(CorpusStatsParams { db_path }): Parameters<CorpusStatsParams>,
    ) -> String {
        match corpus_stats_inner(db_path.as_deref()) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }
}

#[tool_handler(
    name = "mif-mcp",
    version = "0.1.0",
    instructions = "Validate, ingest, and semantically search MIF documents, and resolve MIF \
                    ontology references"
)]
impl ServerHandler for Mif {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = Mif.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        CorpusStatsParams, FindSimilarParams, IngestParams, Mif, Parameters, ResolveParams,
        SearchParams, ValidateParams,
    };

    fn write_temp_file(contents: &str) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), contents).unwrap();
        file
    }

    #[test]
    fn validate_tool_accepts_a_conformant_document() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:test-001",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
        }));
        assert!(result.ends_with(": valid"));
    }

    #[test]
    fn validate_tool_reports_invalid_document_as_problem_json() {
        let file = write_temp_file(r#"{"content": "missing required fields"}"#);
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://mif-spec.dev/errors/invalid-document/v1"
        );
        assert_eq!(value["status"], 422);
    }

    #[test]
    fn validate_tool_reports_missing_file_as_problem_json() {
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: "/nonexistent/mif-mcp-test-fixture.json".into(),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["type"], "https://mif-spec.dev/errors/io/v1");
        assert_eq!(value["status"], 404);
        assert_eq!(value["suggested_fix"]["applicability"], "maybe_incorrect");
    }

    #[test]
    fn validate_tool_reports_a_directory_io_fault_at_500() {
        // Reading a directory as if it were a file is a genuine I/O fault,
        // not a mistaken path — it must stay at 500, not be misclassified
        // as the same 4xx "wrong path" case as a missing file.
        let dir = tempfile::tempdir().unwrap();
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: dir.path().to_path_buf(),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["type"], "https://mif-spec.dev/errors/io/v1");
        assert_eq!(value["status"], 500);
        assert_eq!(value["suggested_fix"]["applicability"], "unspecified");
    }

    #[test]
    fn validate_tool_reports_invalid_json_as_problem_json() {
        let file = write_temp_file("not json");
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["type"], "https://mif-spec.dev/errors/invalid-json/v1");
    }

    #[test]
    fn resolve_tool_returns_the_extends_chain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("mif-base.yaml"),
            "ontology:\n  id: mif-base\n  version: 1.0.0\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("domain.yaml"),
            "ontology:\n  id: domain\n  version: 1.0.0\n  extends: [mif-base]\n",
        )
        .unwrap();
        let result = Mif.resolve_ontology_reference(Parameters(ResolveParams {
            id: "domain".to_string(),
            ontologies_dir: dir.path().to_path_buf(),
        }));
        assert_eq!(result, "mif-base (1.0.0) -> domain (1.0.0)");
    }

    #[test]
    fn resolve_tool_reports_unknown_ontology_as_problem_json() {
        let dir = tempfile::tempdir().unwrap();
        let result = Mif.resolve_ontology_reference(Parameters(ResolveParams {
            id: "missing".to_string(),
            ontologies_dir: dir.path().to_path_buf(),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://mif-spec.dev/errors/ontology-not-found/v1"
        );
        assert_eq!(value["status"], 404);
    }

    const VALID_MARKDOWN_FIXTURE: &str = "---
id: memory:mcp-test-001
type: semantic
created: 2026-07-02T00:00:00Z
---

Test content via MCP.
";

    #[test]
    fn ingest_tool_accepts_a_conformant_document_and_stores_it() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");

        let result = Mif.ingest_mif_document(Parameters(IngestParams {
            file: file.path().to_path_buf(),
            db_path: Some(db_path.clone()),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["lint"], "ok");
        assert_eq!(value["validate"], "ok");
        assert_eq!(value["roundtrip"], "lossless");
        assert_eq!(value["embedding_dim"], 384);
        assert_eq!(value["stored"], true);

        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn ingest_tool_reports_invalid_document_as_problem_json_and_writes_no_row() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        let invalid_file = write_temp_file(
            "---
id: memory:mcp-test-002
created: 2026-07-02T00:00:00Z
---

No type field.
",
        );

        let result = Mif.ingest_mif_document(Parameters(IngestParams {
            file: invalid_file.path().to_path_buf(),
            db_path: Some(db_path.clone()),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://mif-spec.dev/errors/invalid-document/v1"
        );

        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 0);
    }

    fn ingest_fixture(db_path: &std::path::Path, id: &str, content: &str) {
        let file = write_temp_file(&format!(
            "---\nid: {id}\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\n---\n\n{content}\n"
        ));
        Mif.ingest_mif_document(Parameters(IngestParams {
            file: file.path().to_path_buf(),
            db_path: Some(db_path.to_path_buf()),
        }));
    }

    #[test]
    fn search_tool_ranks_ingested_documents_by_relevance() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        ingest_fixture(&db_path, "mcp:cats", "Cats are small domesticated felines.");
        ingest_fixture(
            &db_path,
            "mcp:finance",
            "Quarterly revenue exceeded analyst expectations.",
        );

        let result = Mif.search_documents(Parameters(SearchParams {
            query: "A furry pet cat".to_string(),
            db_path: Some(db_path),
            limit: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["matches"][0]["id"], "urn:mif:mcp:cats");
    }

    #[test]
    fn find_similar_tool_excludes_the_anchor_document_itself() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        ingest_fixture(&db_path, "mcp:a", "Cats are small domesticated felines.");
        ingest_fixture(&db_path, "mcp:b", "Dogs are loyal domesticated canines.");

        let result = Mif.find_similar_documents(Parameters(FindSimilarParams {
            id: "urn:mif:mcp:a".to_string(),
            db_path: Some(db_path),
            limit: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        let ids: Vec<&str> = value["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert!(!ids.contains(&"urn:mif:mcp:a"));
        assert!(ids.contains(&"urn:mif:mcp:b"));
    }

    #[test]
    fn find_similar_tool_reports_document_not_found_as_problem_json() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        mif_store::VectorStore::open(&db_path).unwrap();

        let result = Mif.find_similar_documents(Parameters(FindSimilarParams {
            id: "urn:mif:mcp:missing".to_string(),
            db_path: Some(db_path),
            limit: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://mif-spec.dev/errors/document-not-found/v1"
        );
        assert_eq!(value["status"], 404);
    }

    #[test]
    fn corpus_stats_tool_reports_count_and_dim() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");

        let empty = Mif.corpus_stats(Parameters(CorpusStatsParams {
            db_path: Some(db_path.clone()),
        }));
        let value: serde_json::Value = serde_json::from_str(&empty).unwrap();
        assert_eq!(value["count"], 0);
        assert!(value["dim"].is_null());

        ingest_fixture(&db_path, "mcp:one", "Some content.");
        let result = Mif.corpus_stats(Parameters(CorpusStatsParams {
            db_path: Some(db_path),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["count"], 1);
        assert_eq!(value["dim"], 384);
    }
}
