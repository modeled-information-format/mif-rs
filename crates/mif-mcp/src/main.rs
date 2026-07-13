//! MCP server for the MIF (Modeled Information Format) ecosystem.
//!
//! Exposes nine operations as MCP tools: `validate_mif_document`,
//! `resolve_ontology_reference`, `ingest_mif_document`,
//! `search_documents`, `find_similar_documents`, `corpus_stats`,
//! `roundtrip_mif_document`, `emit_jsonld_document`, and
//! `emit_markdown_document`. Each is a thin wrapper calling the identical
//! `mif-schema`/`mif-ontology`/`mif-frontmatter`/`mif-embed`/`mif-store`
//! functions `mif-cli` calls — kept deliberately in lockstep rather than
//! diverging.
//!
//! An MCP client is inherently a machine consumer (there is no terminal to
//! detect), so every failure renders as a compact RFC 9457
//! `application/problem+json` envelope via [`mif_problem`] rather than plain
//! text — see [`McpError::to_problem`].

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
    /// Path to the MIF document (markdown with frontmatter, or a JSON-LD
    /// projection) to validate.
    file: PathBuf,
    /// MIF level floor to additionally require (1, 2, or 3). Level 1's
    /// fields are already covered by the canonical schema, so the default
    /// is a plain schema validation.
    level: Option<u8>,
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
    /// Additional `SQLite` vector store database(s) to search alongside
    /// `db_path` (or its default), merge-ranked by cosine similarity into
    /// one result list. Empty or omitted means single-root search,
    /// unchanged from before this parameter existed. A root listed more
    /// than once (the same path given as both `db_path` and in
    /// `extra_db_paths`, or repeated within `extra_db_paths`) is not
    /// deduplicated, so its rows are counted and ranked once per
    /// occurrence.
    #[serde(default)]
    extra_db_paths: Vec<PathBuf>,
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
    /// Additional `SQLite` vector store database(s) to search alongside
    /// `db_path` (or its default). See `search_documents`'
    /// `extra_db_paths` for the exact semantics. `id` is looked up across
    /// every root (`db_path` first, then `extra_db_paths` in order) and
    /// excluded from the merged results wherever it appears.
    #[serde(default)]
    extra_db_paths: Vec<PathBuf>,
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
    /// Additional `SQLite` vector store database(s) to summarize alongside
    /// `db_path` (or its default). See `search_documents`' `extra_db_paths`
    /// for the exact semantics.
    #[serde(default)]
    extra_db_paths: Vec<PathBuf>,
}

/// Parameters for the `roundtrip_mif_document` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct RoundtripParams {
    /// Path to the MIF document (markdown with frontmatter, or a JSON-LD
    /// projection) to check.
    file: PathBuf,
    /// Frontmatter shape to use for standalone JSON-LD input whose
    /// identity fields don't already unambiguously indicate one. Ignored
    /// for markdown input, whose shape is auto-detected. `"v1-canonical"`
    /// (default, the MIF v1.0 authoring convention) or `"pre-projected"`.
    #[serde(default)]
    shape: Option<String>,
}

/// Parameters for the `emit_jsonld_document` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EmitJsonldParams {
    /// Path to the MIF document (markdown with frontmatter, or a JSON-LD
    /// projection) to project.
    file: PathBuf,
    /// Write the JSON-LD projection to this path instead of returning it
    /// inline.
    #[serde(default)]
    out: Option<PathBuf>,
    /// Frontmatter shape to use for standalone JSON-LD input whose
    /// identity fields don't already unambiguously indicate one. Ignored
    /// for markdown input, whose shape is auto-detected. `"v1-canonical"`
    /// (default, the MIF v1.0 authoring convention) or `"pre-projected"`.
    #[serde(default)]
    shape: Option<String>,
}

/// Parameters for the `emit_markdown_document` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EmitMarkdownParams {
    /// Path to the JSON-LD MIF document to project.
    file: PathBuf,
    /// Write the markdown to this path instead of returning it inline.
    #[serde(default)]
    out: Option<PathBuf>,
    /// Frontmatter shape to use when the JSON-LD's identity fields don't
    /// already unambiguously indicate one: `"v1-canonical"` (default, the
    /// MIF v1.0 authoring convention) or `"pre-projected"`.
    #[serde(default)]
    shape: Option<String>,
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
    /// The store root this match came from, when more than one root was
    /// queried (`extra_db_paths` was non-empty). Omitted entirely for
    /// single-root queries, so the JSON shape single-root callers already
    /// depend on is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
}

/// The result of a `search_documents`/`find_similar_documents` query.
#[derive(Debug, serde::Serialize)]
struct SearchResult {
    /// Ranked matches, most similar first.
    matches: Vec<MatchEntry>,
}

/// One additional store root's own statistics, nested under
/// [`CorpusStatsResult::extra_roots`].
#[derive(Debug, serde::Serialize)]
struct RootStats {
    /// The vector store database path.
    db: String,
    /// Total number of embeddings stored at this root.
    count: u64,
    /// The dimensionality of a stored embedding at this root, if non-empty.
    dim: Option<usize>,
}

/// The result of a `corpus_stats` query.
///
/// `count`/`dim`/`db` always describe the primary root (`db_path`, or its
/// default) alone, exactly as they did before `extra_db_paths` existed.
/// `total_count` and `extra_roots` are populated only when `extra_db_paths`
/// was non-empty and omitted entirely otherwise, so a single-root query's
/// JSON output is byte-for-byte unchanged from before this parameter
/// existed.
#[derive(Debug, serde::Serialize)]
struct CorpusStatsResult {
    /// Total number of embeddings stored at the primary root.
    count: u64,
    /// The dimensionality of a stored embedding at the primary root, if
    /// non-empty.
    dim: Option<usize>,
    /// The primary vector store database path used.
    db: String,
    /// The summed count across the primary root and every `extra_db_paths`
    /// root. Only present when `extra_db_paths` was non-empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    total_count: Option<u64>,
    /// Each `extra_db_paths` root's own statistics, in the order given.
    /// Empty (and omitted from the JSON output) for a single-root query.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    extra_roots: Vec<RootStats>,
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
    /// A JSON-LD projection could not be serialized to JSON text.
    #[error("failed to serialize JSON-LD: {source}")]
    JsonSerialize {
        /// The underlying serialization error.
        #[source]
        source: serde_json::Error,
    },
}

impl McpError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::Io { .. } => ProblemMeta {
                slug: "mif-mcp-io",
                version: "v1",
                title: "Failed to read an input file",
                status: 500,
                exit_code: 1,
            },
            Self::Json { .. } => ProblemMeta {
                slug: "mif-mcp-invalid-json",
                version: "v1",
                title: "Input file is not valid JSON",
                status: 400,
                exit_code: 2,
            },
            Self::DocumentNotFound(_) => ProblemMeta {
                slug: "mif-mcp-document-not-found",
                version: "v1",
                title: "No document with the given id has been ingested",
                status: 404,
                exit_code: 3,
            },
            Self::JsonSerialize { .. } => ProblemMeta {
                slug: "mif-mcp-json-serialize-failure",
                version: "v1",
                title: "Failed to serialize JSON-LD to text",
                status: 500,
                exit_code: 1,
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
            Self::Json { .. } | Self::JsonSerialize { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string()),
        }
    }
}

/// Validates one MIF document: projects markdown-with-frontmatter or
/// JSON-LD input to JSON-LD via [`project_to_jsonld`] (proving the
/// markdown <-> JSON-LD round trip is lossless either way), then
/// schema-checks the result against the canonical schema and the requested
/// `level` floor (1, 2, or 3; defaults to 1). Unlike
/// [`ingest_mif_document_inner`], this has no side effects: no embedding
/// model load, no vector store write.
///
/// # Errors
///
/// Returns [`McpError`] if the file cannot be read, is not valid
/// JSON-LD/frontmatter, does not round-trip losslessly, does not conform to
/// the canonical MIF schema, does not satisfy the requested level floor, or
/// `level` is not 1, 2, or 3.
fn validate_mif_document_inner(file: &Path, level: Option<u8>) -> Result<String, McpError> {
    let contents = std::fs::read_to_string(file).map_err(|source| McpError::Io {
        path: file.display().to_string(),
        source,
    })?;
    let jsonld = project_to_jsonld(
        file,
        &contents,
        mif_frontmatter::FrontmatterShape::V1Canonical,
    )?;
    let level = mif_schema::Level::try_from(level.unwrap_or(1))?;
    mif_schema::validate_level(&jsonld, level)?;
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
/// markdown-with-frontmatter (starts with `---`) or already JSON-LD. A
/// leading UTF-8 BOM (common in files saved by some Windows editors) is
/// stripped first so it doesn't defeat that dispatch.
fn project_to_jsonld(
    path: &Path,
    contents: &str,
    shape: mif_frontmatter::FrontmatterShape,
) -> Result<serde_json::Value, McpError> {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    if contents.trim_start().starts_with("---") {
        mif_frontmatter::roundtrip_lossless(contents)?;
        let (frontmatter, body) = mif_frontmatter::parse_markdown(contents)?;
        Ok(mif_frontmatter::md_to_jsonld(&frontmatter, &body)?)
    } else {
        let jsonld: serde_json::Value =
            serde_json::from_str(contents).map_err(|source| McpError::Json {
                path: path.display().to_string(),
                source,
            })?;
        // mif_frontmatter::jsonld_roundtrip_lossless proves the JSON-LD ->
        // markdown -> JSON-LD round trip is genuinely lossless (compares
        // reconstructed values against the original, not merely that the
        // derived markdown is stable under a further cycle — real data
        // loss, like a `timestamp` field with no backing
        // `created`/`modified` to regenerate it from, must surface here).
        mif_frontmatter::jsonld_roundtrip_lossless(&jsonld, shape)?;
        Ok(jsonld)
    }
}

/// A stable, non-cryptographic hash of `contents`, used only to detect
/// whether a document's content changed since it was last ingested. FNV-1a
/// is used rather than `std`'s `DefaultHasher`, whose algorithm is
/// explicitly documented as unstable across Rust versions and would make
/// this "stable" claim false for a value that outlives a single build.
fn content_hash(contents: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in contents.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
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

    let jsonld = project_to_jsonld(
        file,
        &contents,
        mif_frontmatter::FrontmatterShape::V1Canonical,
    )?;
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

/// Resolves an optional primary `db_path` plus zero or more
/// `extra_db_paths` values to the full ordered list of vector store roots
/// to query: `db_path` (defaulting to `.mif/vectors.db` exactly as
/// [`resolve_db_path`] does), followed by every `extra_db_paths` entry in
/// the order given.
fn resolve_db_paths(db_path: Option<&Path>, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![resolve_db_path(db_path)];
    roots.extend(extra.iter().cloned());
    roots
}

/// Converts `mif-store`'s single-root similarity matches into this tool's
/// result shape. `root` is always omitted from the output (see
/// [`MatchEntry::root`]).
///
/// A non-finite score (NaN/±inf, only reachable from a corrupt or
/// zero-magnitude stored vector) is clamped to `0.0` — `serde_json` cannot
/// represent non-finite floats, and letting one through would fail the
/// whole tool call's JSON serialization instead of just that one match.
fn to_search_result(matches: Vec<mif_store::SimilarityMatch>) -> SearchResult {
    SearchResult {
        matches: matches
            .into_iter()
            .map(|m| MatchEntry {
                id: m.id,
                score: if m.score.is_finite() { m.score } else { 0.0 },
                root: None,
            })
            .collect(),
    }
}

/// Converts `mif-store`'s multi-root similarity matches into this tool's
/// result shape, including which root each match came from. Applies the
/// same non-finite-score clamp as [`to_search_result`].
fn to_rooted_search_result(matches: Vec<mif_store::RootedMatch>) -> SearchResult {
    SearchResult {
        matches: matches
            .into_iter()
            .map(|m| MatchEntry {
                id: m.id,
                score: if m.score.is_finite() { m.score } else { 0.0 },
                root: Some(m.root.display().to_string()),
            })
            .collect(),
    }
}

/// Embeds `query` and ranks previously ingested documents by cosine
/// similarity to it.
///
/// When `extra_db_paths` is empty, this queries only the resolved `db_path`
/// root, identically to before this parameter existed. Otherwise it queries
/// `db_path` (or its default) together with every `extra_db_paths` root,
/// merge-ranked by cosine similarity via
/// [`mif_store::multi_root_top_k_similar`].
fn search_documents_inner(
    query: &str,
    db_path: Option<&Path>,
    extra_db_paths: &[PathBuf],
    limit: usize,
) -> Result<SearchResult, McpError> {
    let embedder = mif_embed::Embedder::load()?;
    let vector = embedder.embed(query)?;

    if extra_db_paths.is_empty() {
        let db_path = resolve_db_path(db_path);
        let store = mif_store::VectorStore::open(&db_path)?;
        let matches = store.top_k_similar(&vector, limit)?;
        return Ok(to_search_result(matches));
    }

    let roots = resolve_db_paths(db_path, extra_db_paths);
    let matches = mif_store::multi_root_top_k_similar(&roots, &vector, limit)?;
    Ok(to_rooted_search_result(matches))
}

/// Finds documents similar to an already-ingested one, identified by `id`.
///
/// Follows the same single-root-unless-`extra_db_paths`-given behavior as
/// [`search_documents_inner`]. With more than one root, `id` is looked up
/// across every root (`db_path` first, then `extra_db_paths` in order) and
/// excluded from the merged results wherever it appears across every root,
/// not just the one it was found in — see [`mif_store::RootedMatch`]'s doc
/// comment for why that distinction matters once roots can carry colliding
/// ids.
fn find_similar_documents_inner(
    id: &str,
    db_path: Option<&Path>,
    extra_db_paths: &[PathBuf],
    limit: usize,
) -> Result<SearchResult, McpError> {
    // Request one extra match so excluding the anchor document itself still
    // leaves up to `limit` genuinely-similar results. `saturating_add` avoids
    // an overflow panic (debug builds) / silent wraparound (release builds)
    // if a caller passes `limit = usize::MAX` (MCP `limit` deserializes
    // straight from an untrusted tool call with no bounds check).
    let over_fetch = limit.saturating_add(1);

    if extra_db_paths.is_empty() {
        let db_path = resolve_db_path(db_path);
        let store = mif_store::VectorStore::open(&db_path)?;
        let anchor = store
            .get(id)?
            .ok_or_else(|| McpError::DocumentNotFound(id.to_string()))?;
        let matches: Vec<_> = store
            .top_k_similar(&anchor.vector, over_fetch)?
            .into_iter()
            .filter(|m| m.id != id)
            .take(limit)
            .collect();
        return Ok(to_search_result(matches));
    }

    let roots = resolve_db_paths(db_path, extra_db_paths);
    let (_root, anchor) = mif_store::multi_root_get(&roots, id)?
        .ok_or_else(|| McpError::DocumentNotFound(id.to_string()))?;
    let matches: Vec<_> = mif_store::multi_root_top_k_similar(&roots, &anchor.vector, over_fetch)?
        .into_iter()
        .filter(|m| m.id != id)
        .take(limit)
        .collect();
    Ok(to_rooted_search_result(matches))
}

/// Summarizes the vector store's contents.
///
/// Follows the same single-root-unless-`extra_db_paths`-given behavior as
/// [`search_documents_inner`]. With more than one root, `total_count` and
/// `extra_roots` are populated alongside the primary root's own
/// `count`/`dim`/`db` — see [`CorpusStatsResult`]'s doc comment.
fn corpus_stats_inner(
    db_path: Option<&Path>,
    extra_db_paths: &[PathBuf],
) -> Result<CorpusStatsResult, McpError> {
    if extra_db_paths.is_empty() {
        let db_path = resolve_db_path(db_path);
        let store = mif_store::VectorStore::open(&db_path)?;
        let stats = store.stats()?;
        return Ok(CorpusStatsResult {
            count: stats.count,
            dim: stats.dim,
            db: db_path.display().to_string(),
            total_count: None,
            extra_roots: Vec::new(),
        });
    }

    let roots = resolve_db_paths(db_path, extra_db_paths);
    let multi = mif_store::multi_root_stats(&roots)?;
    // `roots` is `[primary, ...extra_db_paths]` by construction, so
    // `multi.per_root`'s first entry is always the primary root's own
    // stats; `skip(1)` for `extra_roots` never panics even in the
    // unreachable case of an empty `per_root`, unlike indexing or
    // `Iterator::next().expect(..)` would. The primary root is stat'd a
    // second time here (once inside `multi_root_stats`, once standalone)
    // to get its stats without relying on that ordering for anything
    // load-bearing — a cheap trade-off (one extra `SELECT COUNT(*)`) for
    // not needing an `unwrap`/`expect` this crate's lints deny.
    let primary_db_path = resolve_db_path(db_path);
    let primary_stats = mif_store::VectorStore::open(&primary_db_path)?.stats()?;
    let extra_roots = multi
        .per_root
        .into_iter()
        .skip(1)
        .map(|(db, stats)| RootStats {
            db: db.display().to_string(),
            count: stats.count,
            dim: stats.dim,
        })
        .collect();

    Ok(CorpusStatsResult {
        count: primary_stats.count,
        dim: primary_stats.dim,
        db: primary_db_path.display().to_string(),
        total_count: Some(multi.total_count),
        extra_roots,
    })
}

/// Parses an optional `shape` string into a
/// [`mif_frontmatter::FrontmatterShape`], via
/// [`mif_frontmatter::FrontmatterShape::try_from`]. Absent defaults to
/// [`mif_frontmatter::FrontmatterShape::V1Canonical`]; any string other than
/// `"v1-canonical"`/`"pre-projected"` is rejected rather than silently
/// falling back — unlike `mif-cli`'s `--shape`, this parameter is not
/// gated by `clap`, so an MCP caller's typo must surface as an error, not a
/// silently wrong reconstruction.
///
/// # Errors
///
/// Returns [`McpError::Frontmatter`] wrapping
/// [`mif_frontmatter::FrontmatterError::UnknownShape`] for an unrecognized
/// string.
fn parse_shape(shape: Option<&str>) -> Result<mif_frontmatter::FrontmatterShape, McpError> {
    shape.map_or(Ok(mif_frontmatter::FrontmatterShape::V1Canonical), |s| {
        Ok(mif_frontmatter::FrontmatterShape::try_from(s)?)
    })
}

/// Proves a MIF document's markdown <-> JSON-LD round trip is lossless, by
/// running [`project_to_jsonld`]'s existing round-trip proof and discarding
/// the projected result. `shape` only affects standalone JSON-LD input;
/// markdown input's shape is auto-detected and ignores it. Pure: no db, no
/// embedder, deterministic output.
///
/// # Errors
///
/// Returns [`McpError`] if the file cannot be read, is not valid
/// JSON-LD/frontmatter, `shape` is unrecognized, or the document does not
/// round-trip losslessly.
fn roundtrip_mif_document_inner(
    file: &Path,
    shape: mif_frontmatter::FrontmatterShape,
) -> Result<String, McpError> {
    let contents = std::fs::read_to_string(file).map_err(|source| McpError::Io {
        path: file.display().to_string(),
        source,
    })?;
    project_to_jsonld(file, &contents, shape)?;
    Ok(format!("{}: roundtrip lossless", file.display()))
}

/// Projects a MIF document (markdown-with-frontmatter or already JSON-LD)
/// to its canonical JSON-LD form via [`project_to_jsonld`] (proving the
/// round trip is lossless in the process), then returns or writes it.
/// `shape` only affects standalone JSON-LD input; markdown input's shape is
/// auto-detected and ignores it. Pure: no db, no embedder.
///
/// # Errors
///
/// Returns [`McpError`] if the file cannot be read, is not valid
/// JSON-LD/frontmatter, `shape` is unrecognized, the document does not
/// round-trip losslessly, the projection cannot be serialized, or (when
/// `out` is given) the output path cannot be written.
fn emit_jsonld_document_inner(
    file: &Path,
    out: Option<&Path>,
    shape: mif_frontmatter::FrontmatterShape,
) -> Result<String, McpError> {
    let contents = std::fs::read_to_string(file).map_err(|source| McpError::Io {
        path: file.display().to_string(),
        source,
    })?;
    let jsonld = project_to_jsonld(file, &contents, shape)?;
    let pretty = serde_json::to_string_pretty(&jsonld)
        .map_err(|source| McpError::JsonSerialize { source })?;
    if let Some(out) = out {
        std::fs::write(out, format!("{pretty}\n")).map_err(|source| McpError::Io {
            path: out.display().to_string(),
            source,
        })?;
        Ok(format!(
            "{}: wrote JSON-LD to {}",
            file.display(),
            out.display()
        ))
    } else {
        Ok(pretty)
    }
}

/// Projects a JSON-LD MIF document to its canonical
/// markdown-with-frontmatter form, proving the round trip is lossless in
/// the process, then returns or writes it. Pure: no db, no embedder.
///
/// # Errors
///
/// Returns [`McpError`] if the file cannot be read, is not valid JSON, does
/// not round-trip losslessly, or (when `out` is given) the output path
/// cannot be written.
fn emit_markdown_document_inner(
    file: &Path,
    out: Option<&Path>,
    shape: mif_frontmatter::FrontmatterShape,
) -> Result<String, McpError> {
    let contents = std::fs::read_to_string(file).map_err(|source| McpError::Io {
        path: file.display().to_string(),
        source,
    })?;
    // Strip a leading UTF-8 BOM (common in files saved by some Windows
    // editors), matching `project_to_jsonld`'s handling of the same case.
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(&contents);
    let jsonld: serde_json::Value =
        serde_json::from_str(contents).map_err(|source| McpError::Json {
            path: file.display().to_string(),
            source,
        })?;
    let (frontmatter, body) = mif_frontmatter::jsonld_roundtrip_lossless(&jsonld, shape)?;
    let markdown = mif_frontmatter::serialize_markdown(&frontmatter, &body)?;
    if let Some(out) = out {
        std::fs::write(out, &markdown).map_err(|source| McpError::Io {
            path: out.display().to_string(),
            source,
        })?;
        Ok(format!(
            "{}: wrote markdown to {}",
            file.display(),
            out.display()
        ))
    } else {
        Ok(markdown)
    }
}

#[derive(Clone)]
struct Mif;

// rmcp's #[tool] macro requires an instance method (&self receiver) for its
// dispatch mechanism, even though these handlers are stateless.
#[allow(clippy::unused_self)]
#[tool_router]
impl Mif {
    #[tool(
        description = "Validate a MIF document (markdown with frontmatter, or a JSON-LD \
                        projection) against the canonical MIF JSON Schema and an optional \
                        L1/L2/L3 level floor (defaults to 1). No side effects."
    )]
    fn validate_mif_document(
        &self,
        Parameters(ValidateParams { file, level }): Parameters<ValidateParams>,
    ) -> String {
        validate_mif_document_inner(&file, level)
            .unwrap_or_else(|error| error.to_problem().to_json())
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
            extra_db_paths,
            limit,
        }): Parameters<SearchParams>,
    ) -> String {
        match search_documents_inner(
            &query,
            db_path.as_deref(),
            &extra_db_paths,
            limit.unwrap_or(DEFAULT_LIMIT),
        ) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(description = "Find previously ingested documents similar to an already-ingested one")]
    fn find_similar_documents(
        &self,
        Parameters(FindSimilarParams {
            id,
            db_path,
            extra_db_paths,
            limit,
        }): Parameters<FindSimilarParams>,
    ) -> String {
        match find_similar_documents_inner(
            &id,
            db_path.as_deref(),
            &extra_db_paths,
            limit.unwrap_or(DEFAULT_LIMIT),
        ) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(description = "Summary statistics over the vector store")]
    fn corpus_stats(
        &self,
        Parameters(CorpusStatsParams {
            db_path,
            extra_db_paths,
        }): Parameters<CorpusStatsParams>,
    ) -> String {
        match corpus_stats_inner(db_path.as_deref(), &extra_db_paths) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => error.to_problem().to_json(),
        }
    }

    #[tool(
        description = "Prove a MIF document's markdown <-> JSON-LD round trip is lossless. Pure: \
                        no db, no embedder."
    )]
    fn roundtrip_mif_document(
        &self,
        Parameters(RoundtripParams { file, shape }): Parameters<RoundtripParams>,
    ) -> String {
        parse_shape(shape.as_deref())
            .and_then(|shape| roundtrip_mif_document_inner(&file, shape))
            .unwrap_or_else(|error| error.to_problem().to_json())
    }

    #[tool(
        description = "Project a MIF document to its canonical JSON-LD form, proving the round \
                        trip is lossless in the process. Pure: no db, no embedder."
    )]
    fn emit_jsonld_document(
        &self,
        Parameters(EmitJsonldParams { file, out, shape }): Parameters<EmitJsonldParams>,
    ) -> String {
        parse_shape(shape.as_deref())
            .and_then(|shape| emit_jsonld_document_inner(&file, out.as_deref(), shape))
            .unwrap_or_else(|error| error.to_problem().to_json())
    }

    #[tool(description = "Project a JSON-LD MIF document to its canonical \
                        markdown-with-frontmatter form, proving the round trip is lossless in \
                        the process. Pure: no db, no embedder.")]
    fn emit_markdown_document(
        &self,
        Parameters(EmitMarkdownParams { file, out, shape }): Parameters<EmitMarkdownParams>,
    ) -> String {
        parse_shape(shape.as_deref())
            .and_then(|shape| emit_markdown_document_inner(&file, out.as_deref(), shape))
            .unwrap_or_else(|error| error.to_problem().to_json())
    }
}

#[tool_handler(
    name = "mif-mcp",
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

    use mif_problem::ToProblem;

    use super::{
        CorpusStatsParams, EmitJsonldParams, EmitMarkdownParams, FindSimilarParams, IngestParams,
        McpError, Mif, Parameters, ResolveParams, RoundtripParams, SearchParams, ValidateParams,
        ingest_mif_document_inner, to_search_result,
    };

    fn write_temp_file(contents: &str) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), contents).unwrap();
        file
    }

    // See the identical helper in mif-cli's test module: cargo test runs
    // tests in parallel, and every test below that ingests or searches
    // loads the embedding model. On a cold cache each load races the others
    // to download and lock the same model blob, which is not reliably
    // concurrent across platforms. Warming the cache once, serialized
    // through `Once`, avoids the race entirely.
    fn warm_embedding_model_cache() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = mif_embed::Embedder::load();
        });
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
            level: None,
        }));
        assert!(result.ends_with(": valid"));
    }

    #[test]
    fn validate_tool_accepts_a_conformant_markdown_document() {
        // Regression test for mif-rs#39: `validate_mif_document` must
        // accept markdown-with-frontmatter directly. Its code path never
        // references `mif_embed`/`mif_store`, so the absence of side
        // effects (unlike `ingest_mif_document`) is a structural
        // property, not one this test runs an assertion against.
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
            level: None,
        }));
        assert!(result.ends_with(": valid"));
    }

    #[test]
    fn validate_tool_accepts_a_markdown_document_with_a_leading_byte_order_mark() {
        // Regression test: a leading UTF-8 BOM (common in files saved by
        // some Windows editors) must not defeat project_to_jsonld's
        // markdown-vs-JSON-LD dispatch.
        let file = write_temp_file(&format!("\u{feff}{VALID_MARKDOWN_FIXTURE}"));
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
            level: None,
        }));
        assert!(result.ends_with(": valid"));
    }

    #[test]
    fn validate_tool_reports_invalid_document_as_problem_json() {
        let file = write_temp_file(r#"{"content": "missing required fields"}"#);
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
            level: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-document/v1"
        );
        assert_eq!(value["status"], 422);
    }

    #[test]
    fn validate_tool_reports_a_level_floor_violation_as_problem_json() {
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
            level: Some(2),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/level-floor-violation/v1"
        );
        assert_eq!(value["status"], 422);
    }

    #[test]
    fn validate_tool_accepts_a_document_satisfying_the_l2_floor() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:test-001",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z",
                "namespace": "test",
                "modified": "2026-07-02T00:00:00Z",
                "temporal": {}
            }"#,
        );
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
            level: Some(2),
        }));
        assert!(result.ends_with(": valid"));
    }

    #[test]
    fn validate_tool_reports_an_out_of_range_level_as_problem_json() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
            level: Some(9),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/unsupported-level/v1"
        );
        assert_eq!(value["status"], 400);
    }

    #[test]
    fn validate_tool_reports_missing_file_as_problem_json() {
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: "/nonexistent/mif-mcp-test-fixture.json".into(),
            level: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-mcp-io/v1"
        );
        assert_eq!(value["status"], 404);
        assert_eq!(value["suggested_fix"]["applicability"], "maybe_incorrect");
    }

    #[test]
    fn validate_tool_reports_a_directory_io_fault_at_500() {
        // Reading a directory as if it were a file is a genuine I/O fault,
        // not a mistaken path — on Unix this must stay at 500, not be
        // misclassified as the same 4xx "wrong path" case as a missing
        // file. Windows genuinely reports this differently: opening a
        // directory for read access fails at the OS level with "access
        // denied", which `std::io` surfaces as `ErrorKind::PermissionDenied`
        // — the same kind a real permissions fault would produce — so
        // `classify_io_error` cannot tell the two apart there and correctly
        // classifies it as the 403 "maybe incorrect" case instead.
        let dir = tempfile::tempdir().unwrap();
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: dir.path().to_path_buf(),
            level: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-mcp-io/v1"
        );
        #[cfg(not(windows))]
        {
            assert_eq!(value["status"], 500);
            assert_eq!(value["suggested_fix"]["applicability"], "unspecified");
        }
        #[cfg(windows)]
        {
            assert_eq!(value["status"], 403);
            assert_eq!(value["suggested_fix"]["applicability"], "maybe_incorrect");
        }
    }

    #[test]
    fn validate_tool_reports_invalid_json_as_problem_json() {
        let file = write_temp_file("not json");
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
            level: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-mcp-invalid-json/v1"
        );
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
            "https://modeled-information-format.github.io/mif-rs/references/errors/ontology-not-found/v1"
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

    // See the identical fixture and its rationale in mif-cli's test module.
    const DRIFTING_MARKDOWN_FIXTURE: &str =
        "---\nid: x\ntype: semantic\n123: orphaned-value\n---\n\nBody.\n";

    #[test]
    fn roundtrip_tool_accepts_a_conformant_document() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let result = Mif.roundtrip_mif_document(Parameters(RoundtripParams {
            file: file.path().to_path_buf(),
            shape: None,
        }));
        assert!(result.ends_with("roundtrip lossless"));
    }

    #[test]
    fn roundtrip_tool_reports_a_drifting_document_as_problem_json() {
        let file = write_temp_file(DRIFTING_MARKDOWN_FIXTURE);
        let result = Mif.roundtrip_mif_document(Parameters(RoundtripParams {
            file: file.path().to_path_buf(),
            shape: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/roundtrip-drift/v1"
        );
    }

    #[test]
    fn roundtrip_tool_reports_json_ld_field_loss_as_problem_json() {
        // See the identical regression in mif-cli's test module: a
        // `timestamp` field with no backing `created`/`modified` to
        // regenerate it from must surface as drift, not a false "lossless"
        // report.
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:mcp-timestamp-loss-test",
                "conceptType": "semantic",
                "content": "Test content.",
                "timestamp": "2026-01-01T00:00:00Z"
            }"#,
        );
        let result = Mif.roundtrip_mif_document(Parameters(RoundtripParams {
            file: file.path().to_path_buf(),
            shape: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/roundtrip-drift/v1"
        );
    }

    #[test]
    fn roundtrip_tool_accepts_json_ld_whose_timestamp_is_consistently_derived() {
        // Non-regression counterpart: must not false-positive on a
        // schema-valid document.
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:mcp-timestamp-consistent-test",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-01-01T00:00:00Z",
                "timestamp": "2026-01-01T00:00:00Z"
            }"#,
        );
        let result = Mif.roundtrip_mif_document(Parameters(RoundtripParams {
            file: file.path().to_path_buf(),
            shape: None,
        }));
        assert!(result.ends_with("roundtrip lossless"));
    }

    #[test]
    fn roundtrip_tool_reports_an_unrecognized_shape_as_problem_json() {
        // Unlike mif-cli's clap-gated `--shape`, this MCP parameter is not
        // pre-validated — an unrecognized string must surface as an error,
        // not silently fall back to a default (the asymmetry a prior
        // version of this tool had).
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let result = Mif.roundtrip_mif_document(Parameters(RoundtripParams {
            file: file.path().to_path_buf(),
            shape: Some("PreProjected".to_string()),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/unknown-frontmatter-shape/v1"
        );
    }

    #[test]
    fn emit_jsonld_tool_returns_the_projection_inline_by_default() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let result = Mif.emit_jsonld_document(Parameters(EmitJsonldParams {
            file: file.path().to_path_buf(),
            out: None,
            shape: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["@id"], "urn:mif:memory:mcp-test-001");
    }

    #[test]
    fn emit_jsonld_tool_writes_to_the_out_path_when_given() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("out.json");
        let result = Mif.emit_jsonld_document(Parameters(EmitJsonldParams {
            file: file.path().to_path_buf(),
            out: Some(out_path.clone()),
            shape: None,
        }));
        assert!(result.contains("wrote JSON-LD to"));
        let written = fs::read_to_string(&out_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value["@id"], "urn:mif:memory:mcp-test-001");
    }

    #[test]
    fn emit_markdown_tool_returns_the_projection_inline_by_default() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:mcp-emit-md-test",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let result = Mif.emit_markdown_document(Parameters(EmitMarkdownParams {
            file: file.path().to_path_buf(),
            out: None,
            shape: None,
        }));
        assert!(result.starts_with("---\n"));
    }

    #[test]
    fn emit_markdown_tool_accepts_json_ld_with_a_leading_byte_order_mark() {
        // See the identical regression in mif-cli's test module.
        let mut contents = "\u{feff}".to_string();
        contents.push_str(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:mcp-bom-test",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let file = write_temp_file(&contents);
        let result = Mif.emit_markdown_document(Parameters(EmitMarkdownParams {
            file: file.path().to_path_buf(),
            out: None,
            shape: None,
        }));
        assert!(result.starts_with("---\n"));
    }

    #[test]
    fn emit_markdown_tool_reports_invalid_json_as_problem_json() {
        let file = write_temp_file("not json");
        let result = Mif.emit_markdown_document(Parameters(EmitMarkdownParams {
            file: file.path().to_path_buf(),
            out: None,
            shape: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-mcp-invalid-json/v1"
        );
    }

    #[test]
    fn emit_markdown_tool_reports_json_ld_field_loss_as_problem_json() {
        // See the identical regression in mif-cli's test module.
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:mcp-emit-md-timestamp-loss-test",
                "conceptType": "semantic",
                "content": "Test content.",
                "timestamp": "2026-01-01T00:00:00Z"
            }"#,
        );
        let result = Mif.emit_markdown_document(Parameters(EmitMarkdownParams {
            file: file.path().to_path_buf(),
            out: None,
            shape: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/roundtrip-drift/v1"
        );
    }

    #[test]
    fn json_serialize_error_maps_to_a_versioned_problem_type() {
        let source = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let error = McpError::JsonSerialize { source };
        let problem = error.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-mcp-json-serialize-failure/v1"
        );
        assert_eq!(problem.status, 500);
        assert_eq!(problem.exit_code, Some(1));
    }

    #[test]
    fn ingest_tool_accepts_a_conformant_document_and_stores_it() {
        warm_embedding_model_cache();
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
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-document/v1"
        );

        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn ingest_reports_the_real_file_path_on_a_json_ld_parse_error() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        let file = write_temp_file("not valid json");

        let error = ingest_mif_document_inner(file.path(), Some(&db_path)).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(&file.path().display().to_string()),
            "expected the real file path in {message:?}, not the ingest-input placeholder"
        );
    }

    #[test]
    fn delegated_error_variants_render_a_sane_problem_if_ever_directly_matched() {
        // See the identical rationale in mif-cli's test module: `meta()`'s
        // Schema/Ontology/Frontmatter/Embed/Store arm is dead in practice
        // but exists as a defensive fallback — exercise it directly.
        for error in [
            McpError::Frontmatter(mif_frontmatter::FrontmatterError::MissingFrontmatter),
            McpError::Embed(mif_embed::EmbedError::NoCacheDir { model: "test" }),
            McpError::Store(mif_store::StoreError::MissingParentDir {
                path: "test".to_string(),
            }),
        ] {
            let problem = error.to_problem();
            assert!(problem.status >= 400, "status was {}", problem.status);
            // `to_problem()` never actually calls `meta()` for these
            // variants (it delegates to the inner error instead), so
            // `meta()`'s own fallback arm needs a direct call to cover.
            let meta = error.meta();
            assert_eq!(meta.status, 500);
        }
    }

    #[test]
    fn ingest_tool_missing_file_reports_a_404_problem() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        let error = ingest_mif_document_inner(
            std::path::Path::new("/nonexistent/mif-mcp-fixture.json"),
            Some(&db_path),
        )
        .unwrap_err();
        assert_eq!(error.to_problem().status, 404);
    }

    #[test]
    fn ingest_tool_reports_an_io_error_when_the_db_parent_directory_cannot_be_created() {
        warm_embedding_model_cache();
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let parent_dir = tempfile::tempdir().unwrap();
        let blocker = parent_dir.path().join("blocker");
        fs::write(&blocker, "not a directory").unwrap();
        let db_path = blocker.join("subdir").join("vectors.db");

        let error = ingest_mif_document_inner(file.path(), Some(&db_path)).unwrap_err();
        assert_eq!(error.to_problem().status, 500);
    }

    #[test]
    fn search_tool_reports_a_problem_when_the_store_cannot_be_opened() {
        // A directory can't be opened as a SQLite database file.
        let db_dir = tempfile::tempdir().unwrap();
        let result = Mif.search_documents(Parameters(SearchParams {
            query: "anything".to_string(),
            db_path: Some(db_dir.path().to_path_buf()),
            extra_db_paths: Vec::new(),
            limit: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(value.get("status").is_some());
    }

    #[test]
    fn corpus_stats_tool_reports_a_problem_when_the_store_cannot_be_opened() {
        let db_dir = tempfile::tempdir().unwrap();
        let result = Mif.corpus_stats(Parameters(CorpusStatsParams {
            db_path: Some(db_dir.path().to_path_buf()),
            extra_db_paths: Vec::new(),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(value.get("status").is_some());
    }

    #[test]
    fn to_search_result_clamps_a_non_finite_score_to_zero() {
        let result = to_search_result(vec![
            mif_store::SimilarityMatch {
                id: "urn:mif:memory:a".to_string(),
                score: f32::NAN,
            },
            mif_store::SimilarityMatch {
                id: "urn:mif:memory:b".to_string(),
                score: f32::INFINITY,
            },
        ]);
        // A non-finite score would otherwise fail serde_json::to_string
        // for the whole result; assert it serializes cleanly at 0.0.
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"score\":0.0"));
    }

    fn ingest_fixture(db_path: &std::path::Path, id: &str, content: &str) {
        warm_embedding_model_cache();
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
            extra_db_paths: Vec::new(),
            limit: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["matches"][0]["id"], "urn:mif:mcp:cats");
        assert!(value["matches"][0]["root"].is_null());
    }

    #[test]
    fn search_tool_with_extra_db_paths_merges_and_ranks_across_roots() {
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");
        ingest_fixture(
            &db_path_a,
            "mcp:cats",
            "Cats are small domesticated felines.",
        );
        ingest_fixture(
            &db_path_b,
            "mcp:finance",
            "Quarterly revenue exceeded analyst expectations.",
        );

        let result = Mif.search_documents(Parameters(SearchParams {
            query: "A furry pet cat".to_string(),
            db_path: Some(db_path_a.clone()),
            extra_db_paths: vec![db_path_b],
            limit: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        let matches = value["matches"].as_array().unwrap();
        assert_eq!(matches[0]["id"], "urn:mif:mcp:cats");
        assert_eq!(matches[0]["root"], db_path_a.display().to_string());
        assert!(
            matches
                .iter()
                .any(|m| m["id"].as_str().unwrap() == "urn:mif:mcp:finance")
        );
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
            extra_db_paths: Vec::new(),
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
    fn find_similar_tool_with_extra_db_paths_excludes_the_anchor_from_every_root() {
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");
        ingest_fixture(&db_path_a, "mcp:a", "Cats are small domesticated felines.");
        ingest_fixture(&db_path_b, "mcp:b", "Dogs are loyal domesticated canines.");

        let result = Mif.find_similar_documents(Parameters(FindSimilarParams {
            id: "urn:mif:mcp:a".to_string(),
            db_path: Some(db_path_a),
            extra_db_paths: vec![db_path_b],
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
            extra_db_paths: Vec::new(),
            limit: None,
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["type"],
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-mcp-document-not-found/v1"
        );
        assert_eq!(value["status"], 404);
    }

    #[test]
    fn corpus_stats_tool_reports_count_and_dim() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");

        let empty = Mif.corpus_stats(Parameters(CorpusStatsParams {
            db_path: Some(db_path.clone()),
            extra_db_paths: Vec::new(),
        }));
        let value: serde_json::Value = serde_json::from_str(&empty).unwrap();
        assert_eq!(value["count"], 0);
        assert!(value["dim"].is_null());
        assert!(value["total_count"].is_null());
        // `extra_roots` is skip-serialized when empty (see
        // `CorpusStatsResult::extra_roots`'s doc comment), so a single-root
        // query's JSON has no `extra_roots` key at all rather than `[]`.
        assert!(value["extra_roots"].is_null());

        ingest_fixture(&db_path, "mcp:one", "Some content.");
        let result = Mif.corpus_stats(Parameters(CorpusStatsParams {
            db_path: Some(db_path),
            extra_db_paths: Vec::new(),
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["count"], 1);
        assert_eq!(value["dim"], 384);
    }

    #[test]
    fn corpus_stats_tool_with_extra_db_paths_reports_a_total_and_a_per_root_breakdown() {
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");
        ingest_fixture(&db_path_a, "mcp:one", "Some content.");
        ingest_fixture(&db_path_b, "mcp:two", "Other content.");
        ingest_fixture(&db_path_b, "mcp:three", "More content.");

        let result = Mif.corpus_stats(Parameters(CorpusStatsParams {
            db_path: Some(db_path_a.clone()),
            extra_db_paths: vec![db_path_b.clone()],
        }));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["count"], 1);
        assert_eq!(value["dim"], 384);
        assert_eq!(value["db"], db_path_a.display().to_string());
        assert_eq!(value["total_count"], 3);
        let extra_roots = value["extra_roots"].as_array().unwrap();
        assert_eq!(extra_roots.len(), 1);
        assert_eq!(extra_roots[0]["db"], db_path_b.display().to_string());
        assert_eq!(extra_roots[0]["count"], 2);
        assert_eq!(extra_roots[0]["dim"], 384);
    }
}
