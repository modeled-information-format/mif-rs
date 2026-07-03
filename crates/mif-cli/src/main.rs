//! Command-line interface for the MIF (Modeled Information Format) ecosystem.
//!
//! A CLI naturally writes to stdout/stderr; this binary exempts itself from
//! the workspace's `print_stdout`/`print_stderr` lints for that reason (see
//! this repo's `CLAUDE.md`, "Lint Configuration").
//!
//! Errors are reported through [`mif_problem`]'s RFC 9457 pattern: pretty
//! text by default on a terminal, or a compact `application/problem+json`
//! envelope when `--format json` is given or stderr is not a terminal (see
//! [`mif_problem::OutputFormat::select`]).
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mif_problem::{OutputFormat, ProblemMeta, ToProblem};

/// Default path for the vector store database, relative to the current
/// working directory, when `--db-path` is not given.
const DEFAULT_DB_PATH: &str = ".mif/vectors.db";

#[derive(Parser)]
#[command(
    name = "mif-cli",
    version,
    about = "CLI for the MIF (Modeled Information Format) ecosystem"
)]
struct Cli {
    /// Error rendering format. Defaults to `pretty` on a terminal and `json`
    /// otherwise.
    #[arg(long, global = true, value_parser = ["pretty", "json"])]
    format: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a MIF document against the canonical schema.
    Validate {
        /// Path to the MIF document (JSON-LD projection) to validate.
        file: PathBuf,
    },
    /// Ontology-related operations.
    Ontology {
        #[command(subcommand)]
        command: OntologyCommand,
    },
    /// Lint, validate, prove a lossless round trip, compute an embedding,
    /// and store the embedding vector for one MIF document.
    Ingest {
        /// Path to the MIF document (markdown with frontmatter, or a
        /// JSON-LD projection) to ingest.
        file: PathBuf,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`, created (along with its parent directory) if
        /// absent.
        #[arg(long)]
        db_path: Option<PathBuf>,
    },
    /// Free-text semantic search over previously ingested documents.
    Search {
        /// The query text to embed and rank stored documents against.
        query: String,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
        /// Maximum number of ranked results to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Find previously ingested documents similar to an already-ingested one.
    FindSimilar {
        /// The id of an already-ingested document (as reported by `ingest`).
        id: String,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
        /// Maximum number of ranked results to return (excluding `id`
        /// itself).
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Summary statistics over the vector store.
    CorpusStats {
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum OntologyCommand {
    /// Resolve an ontology's three-tier `extends` chain.
    Resolve {
        /// The ontology ID to resolve.
        id: String,
        /// Directory containing ontology definition YAML files.
        #[arg(long)]
        ontologies_dir: PathBuf,
    },
}

/// Errors reported by the `mif-cli` binary itself.
#[derive(Debug, thiserror::Error)]
enum CliError {
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
    /// A `find-similar` query named an id that has never been ingested.
    #[error("no document with id '{0}' has been ingested into this vector store")]
    DocumentNotFound(String),
}

impl CliError {
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
            // `ProblemMeta` internally; these arms are only reached if
            // `to_problem` needs a CLI-level fallback, which it does not
            // (see `to_problem` below).
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

impl ToProblem for CliError {
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
                    "Ingest the document first with `mif-cli ingest`, or check the id for a \
                     typo.",
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = OutputFormat::select(cli.format.as_deref(), std::io::stderr().is_terminal());
    match run(&cli.command) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        },
        Err(error) => {
            eprintln!("{}", error.render(format));
            ExitCode::from(error.to_problem().exit_code.unwrap_or(1))
        },
    }
}

fn run(command: &Command) -> Result<String, CliError> {
    match command {
        Command::Validate { file } => validate(file),
        Command::Ontology { command } => match command {
            OntologyCommand::Resolve { id, ontologies_dir } => resolve(id, ontologies_dir),
        },
        Command::Ingest { file, db_path } => ingest(file, db_path.as_deref()),
        Command::Search {
            query,
            db_path,
            limit,
        } => search(query, db_path.as_deref(), *limit),
        Command::FindSimilar { id, db_path, limit } => find_similar(id, db_path.as_deref(), *limit),
        Command::CorpusStats { db_path } => corpus_stats(db_path.as_deref()),
    }
}

/// Resolves an optional `--db-path` to the effective vector store path.
fn resolve_db_path(db_path: Option<&Path>) -> PathBuf {
    db_path.map_or_else(|| PathBuf::from(DEFAULT_DB_PATH), Path::to_path_buf)
}

/// Formats a ranked similarity result list for display, one line per match.
fn format_matches(matches: &[mif_store::SimilarityMatch]) -> String {
    if matches.is_empty() {
        return "(no matches)".to_string();
    }
    matches
        .iter()
        .map(|m| format!("{:.4}  {}", m.score, m.id))
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate(file: &Path) -> Result<String, CliError> {
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })?;
    let instance: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| CliError::Json {
            path: file.display().to_string(),
            source,
        })?;
    mif_schema::validate_document(&instance)?;
    Ok(format!("{}: valid", file.display()))
}

fn resolve(id: &str, ontologies_dir: &Path) -> Result<String, CliError> {
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
fn project_to_jsonld(path: &Path, contents: &str) -> Result<serde_json::Value, CliError> {
    if contents.trim_start().starts_with("---") {
        mif_frontmatter::roundtrip_lossless(contents)?;
        let (frontmatter, body) = mif_frontmatter::parse_markdown(contents)?;
        Ok(mif_frontmatter::md_to_jsonld(&frontmatter, &body)?)
    } else {
        let jsonld: serde_json::Value =
            serde_json::from_str(contents).map_err(|source| CliError::Json {
                path: path.display().to_string(),
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
///
/// # Errors
///
/// Returns [`CliError`] if the file cannot be read, does not conform to the
/// canonical MIF schema, does not round-trip losslessly, the embedding model
/// cannot be loaded or run, or the vector store cannot be opened or written.
fn ingest(file: &Path, db_path: Option<&Path>) -> Result<String, CliError> {
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })?;

    let jsonld = project_to_jsonld(file, &contents)?;
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
        std::fs::create_dir_all(parent).map_err(|source| CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let store = mif_store::VectorStore::open(&db_path)?;
    let hash = content_hash(&contents);
    let updated_at = chrono::Utc::now().to_rfc3339();
    store.upsert(&id, &vector, &hash, &updated_at)?;

    Ok(format!(
        "{}: lint=ok validate=ok roundtrip=lossless embedding_dim={} stored=true (id={id}, db={})",
        file.display(),
        vector.len(),
        db_path.display()
    ))
}

/// Embeds `query` and ranks previously ingested documents by cosine
/// similarity to it.
///
/// # Errors
///
/// Returns [`CliError`] if the embedding model cannot be loaded or run, or
/// the vector store cannot be opened or queried.
fn search(query: &str, db_path: Option<&Path>, limit: usize) -> Result<String, CliError> {
    let embedder = mif_embed::Embedder::load()?;
    let vector = embedder.embed(query)?;

    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let matches = store.top_k_similar(&vector, limit)?;

    Ok(format_matches(&matches))
}

/// Finds documents similar to an already-ingested one, identified by `id`.
///
/// # Errors
///
/// Returns [`CliError::DocumentNotFound`] if `id` has never been ingested
/// into this store, or [`CliError`] if the vector store cannot be opened or
/// queried.
fn find_similar(id: &str, db_path: Option<&Path>, limit: usize) -> Result<String, CliError> {
    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let anchor = store
        .get(id)?
        .ok_or_else(|| CliError::DocumentNotFound(id.to_string()))?;

    // Request one extra match so excluding the anchor document itself still
    // leaves up to `limit` genuinely-similar results. `saturating_add` avoids
    // an overflow panic (debug builds) / silent wraparound (release builds)
    // if a caller passes `limit = usize::MAX`.
    let matches: Vec<_> = store
        .top_k_similar(&anchor.vector, limit.saturating_add(1))?
        .into_iter()
        .filter(|m| m.id != id)
        .take(limit)
        .collect();

    Ok(format_matches(&matches))
}

/// Summarizes the vector store's contents.
///
/// # Errors
///
/// Returns [`CliError`] if the vector store cannot be opened or queried.
fn corpus_stats(db_path: Option<&Path>) -> Result<String, CliError> {
    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let stats = store.stats()?;

    Ok(stats.dim.map_or_else(
        || format!("count=0 db={}", db_path.display()),
        |dim| format!("count={} dim={dim} db={}", stats.count, db_path.display()),
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use mif_problem::{OutputFormat, ToProblem};

    use super::{
        CliError, Command, OntologyCommand, corpus_stats, find_similar, ingest, resolve, run,
        search, validate,
    };

    fn write_temp_file(contents: &str) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), contents).unwrap();
        file
    }

    // `cargo test` runs tests in parallel threads within one process. Every
    // test below that ingests or searches loads the embedding model, and on
    // a cold cache each load races the others to download and lock the same
    // model blob — `hf-hub`'s lock acquisition is not reliably concurrent
    // across platforms (observed failing on macOS/Windows CI, passing on
    // Linux). Warming the cache once, serialized through `Once`, means every
    // real `Embedder::load()` call below hits an already-populated cache and
    // never contends on the download lock.
    fn warm_embedding_model_cache() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = mif_embed::Embedder::load();
        });
    }

    #[test]
    fn validate_accepts_a_conformant_document() {
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
        assert_eq!(
            validate(file.path()).unwrap(),
            format!("{}: valid", file.path().display())
        );
    }

    #[test]
    fn validate_rejects_a_non_conformant_document() {
        let file = write_temp_file(r#"{"content": "missing required fields"}"#);
        let error = validate(file.path()).unwrap_err();
        assert!(error.to_string().contains("failed schema validation"));
    }

    #[test]
    fn validate_reports_invalid_json() {
        let file = write_temp_file("not json");
        let error = validate(file.path()).unwrap_err();
        assert!(error.to_string().contains("failed to parse"));
    }

    #[test]
    fn validate_reports_missing_file() {
        let missing = std::path::Path::new("/nonexistent/mif-cli-test-fixture.json");
        let error = validate(missing).unwrap_err();
        assert!(error.to_string().contains("failed to read"));
    }

    #[test]
    fn resolve_prints_the_extends_chain() {
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
        assert_eq!(
            resolve("domain", dir.path()).unwrap(),
            "mif-base (1.0.0) -> domain (1.0.0)"
        );
    }

    #[test]
    fn resolve_reports_unknown_ontology() {
        let dir = tempfile::tempdir().unwrap();
        let error = resolve("missing", dir.path()).unwrap_err();
        assert!(error.to_string().contains("not found"));
    }

    #[test]
    fn invalid_document_error_renders_as_problem_json() {
        let file = write_temp_file(r#"{"content": "missing required fields"}"#);
        let error = validate(file.path()).unwrap_err();
        let json = error.render(OutputFormat::Json);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        for member in [
            "type",
            "title",
            "status",
            "detail",
            "instance",
            "retry_after",
        ] {
            assert!(value.get(member).is_some(), "missing {member}");
        }
        assert_eq!(
            value["type"],
            "https://mif-spec.dev/errors/invalid-document/v1"
        );
    }

    #[test]
    fn missing_file_io_error_classifies_as_a_404_maybe_incorrect_problem() {
        let error =
            validate(std::path::Path::new("/nonexistent/mif-cli-fixture.json")).unwrap_err();
        let problem = error.to_problem();
        assert_eq!(problem.status, 404);
        assert_eq!(
            problem.suggested_fix.unwrap().applicability,
            mif_problem::Applicability::MaybeIncorrect
        );
    }

    #[test]
    fn directory_io_error_classifies_as_a_500_unspecified_problem() {
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
        let error = validate(dir.path()).unwrap_err();
        let problem = error.to_problem();
        #[cfg(not(windows))]
        {
            assert_eq!(problem.status, 500);
            assert_eq!(
                problem.suggested_fix.unwrap().applicability,
                mif_problem::Applicability::Unspecified
            );
        }
        #[cfg(windows)]
        {
            assert_eq!(problem.status, 403);
            assert_eq!(
                problem.suggested_fix.unwrap().applicability,
                mif_problem::Applicability::MaybeIncorrect
            );
        }
    }

    #[test]
    fn unknown_ontology_error_renders_as_problem_json() {
        let dir = tempfile::tempdir().unwrap();
        let error = resolve("missing", dir.path()).unwrap_err();
        let problem = error.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://mif-spec.dev/errors/ontology-not-found/v1"
        );
        assert_eq!(problem.status, 404);
    }

    #[test]
    fn pretty_render_matches_error_prefixed_display() {
        let error =
            validate(std::path::Path::new("/nonexistent/mif-cli-fixture.json")).unwrap_err();
        assert_eq!(
            error.render(OutputFormat::Pretty),
            format!("Error: {error}")
        );
    }

    const VALID_MARKDOWN_FIXTURE: &str = "---
id: memory:test-001
type: semantic
created: 2026-07-02T00:00:00Z
---

Test content.
";

    #[test]
    fn ingest_accepts_a_conformant_markdown_document_and_stores_it() {
        warm_embedding_model_cache();
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");

        let message = ingest(file.path(), Some(&db_path)).unwrap();
        assert!(message.contains("lint=ok"));
        assert!(message.contains("validate=ok"));
        assert!(message.contains("roundtrip=lossless"));
        assert!(message.contains("embedding_dim=384"));
        assert!(message.contains("stored=true"));

        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        let stored = store.get("urn:mif:memory:test-001").unwrap().unwrap();
        assert_eq!(stored.dim, 384);
    }

    #[test]
    fn ingest_accepts_a_conformant_jsonld_document() {
        warm_embedding_model_cache();
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:test-002",
                "conceptType": "semantic",
                "content": "Other content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");

        let message = ingest(file.path(), Some(&db_path)).unwrap();
        assert!(message.contains("embedding_dim=384"));

        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert!(store.get("urn:mif:memory:test-002").unwrap().is_some());
    }

    #[test]
    fn ingest_rejects_invalid_document_and_writes_no_row() {
        warm_embedding_model_cache();
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");

        // First, a valid ingest establishes a baseline row count.
        let valid_file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        ingest(valid_file.path(), Some(&db_path)).unwrap();
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 1);

        // An invalid document (missing required `type`/conceptType) must
        // fail before ever touching the store.
        let invalid_file = write_temp_file(
            "---
id: memory:test-003
created: 2026-07-02T00:00:00Z
---

No type field.
",
        );
        let error = ingest(invalid_file.path(), Some(&db_path)).unwrap_err();
        assert!(error.to_string().contains("failed schema validation"));

        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn ingest_invalid_document_renders_as_problem_json() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        let invalid_file = write_temp_file(
            "---
id: memory:test-004
created: 2026-07-02T00:00:00Z
---

No type field.
",
        );

        let error = ingest(invalid_file.path(), Some(&db_path)).unwrap_err();
        let json = error.render(OutputFormat::Json);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        for member in [
            "type",
            "title",
            "status",
            "detail",
            "instance",
            "retry_after",
            "suggested_fix",
            "code_actions",
        ] {
            assert!(value.get(member).is_some(), "missing {member}");
        }
        assert_eq!(
            value["type"],
            "https://mif-spec.dev/errors/invalid-document/v1"
        );
    }

    #[test]
    fn ingest_missing_file_classifies_as_a_404_maybe_incorrect_problem() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        let error = ingest(
            std::path::Path::new("/nonexistent/mif-cli-fixture.json"),
            Some(&db_path),
        )
        .unwrap_err();
        assert_eq!(error.to_problem().status, 404);
    }

    #[test]
    fn ingest_reports_an_io_error_when_the_db_parent_directory_cannot_be_created() {
        warm_embedding_model_cache();
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        // `blocker` exists as a plain file, so `create_dir_all` on a path
        // that treats it as an intermediate directory component must fail.
        let parent_dir = tempfile::tempdir().unwrap();
        let blocker = parent_dir.path().join("blocker");
        fs::write(&blocker, "not a directory").unwrap();
        let db_path = blocker.join("subdir").join("vectors.db");

        let error = ingest(file.path(), Some(&db_path)).unwrap_err();
        assert_eq!(error.to_problem().status, 500);
    }

    #[test]
    fn ingest_reports_the_real_file_path_on_a_json_ld_parse_error() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        let file = write_temp_file("not valid json");

        let error = ingest(file.path(), Some(&db_path)).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(&file.path().display().to_string()),
            "expected the real file path in {message:?}, not the ingest-input placeholder"
        );
    }

    #[test]
    fn exit_code_reflects_the_mapped_problem_type_not_a_flat_failure() {
        let io_error = CliError::Io {
            path: "x".to_string(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert_eq!(io_error.to_problem().exit_code, Some(1));

        let json_error = CliError::Json {
            path: "x".to_string(),
            source: serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
        };
        assert_eq!(json_error.to_problem().exit_code, Some(2));

        let not_found = CliError::DocumentNotFound("urn:mif:memory:missing".to_string());
        assert_eq!(not_found.to_problem().exit_code, Some(3));
    }

    #[test]
    fn delegated_error_variants_render_a_sane_problem_if_ever_directly_matched() {
        // `meta()`'s Schema/Ontology/Frontmatter/Embed/Store arm is dead in
        // practice — `to_problem()` always delegates to the inner error
        // instead of calling `meta()` for these variants — but it exists as
        // a defensive fallback. Exercise it directly so that fallback stays
        // provably sane (500, non-panicking) rather than untested.
        for error in [
            CliError::Frontmatter(mif_frontmatter::FrontmatterError::MissingFrontmatter),
            CliError::Embed(mif_embed::EmbedError::NoCacheDir { model: "test" }),
            CliError::Store(mif_store::StoreError::MissingParentDir {
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
    fn run_dispatches_every_subcommand_to_its_handler() {
        let ontologies_dir = tempfile::tempdir().unwrap();
        fs::write(
            ontologies_dir.path().join("base.yaml"),
            "ontology:\n  id: base\n  version: 1.0.0\n",
        )
        .unwrap();
        let resolve_result = run(&Command::Ontology {
            command: OntologyCommand::Resolve {
                id: "base".to_string(),
                ontologies_dir: ontologies_dir.path().to_path_buf(),
            },
        });
        assert!(resolve_result.is_ok());

        let missing_file = tempfile::tempdir().unwrap().path().join("missing.json");
        let validate_result = run(&Command::Validate { file: missing_file });
        assert!(validate_result.is_err());

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        warm_embedding_model_cache();
        let doc_file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let ingest_result = run(&Command::Ingest {
            file: doc_file.path().to_path_buf(),
            db_path: Some(db_path.clone()),
        });
        assert!(ingest_result.is_ok());

        let search_result = run(&Command::Search {
            query: "test content".to_string(),
            db_path: Some(db_path.clone()),
            limit: 5,
        });
        assert!(search_result.is_ok());

        let find_similar_result = run(&Command::FindSimilar {
            id: "urn:mif:memory:test-001".to_string(),
            db_path: Some(db_path.clone()),
            limit: 5,
        });
        assert!(find_similar_result.is_ok());

        let corpus_stats_result = run(&Command::CorpusStats {
            db_path: Some(db_path),
        });
        assert!(corpus_stats_result.is_ok());
    }

    fn ingest_fixture(db_path: &std::path::Path, id: &str, content: &str) {
        warm_embedding_model_cache();
        let file = write_temp_file(&format!(
            "---\nid: {id}\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\n---\n\n{content}\n"
        ));
        ingest(file.path(), Some(db_path)).unwrap();
    }

    #[test]
    fn search_ranks_ingested_documents_by_relevance() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        ingest_fixture(
            &db_path,
            "memory:cats",
            "Cats are small domesticated felines.",
        );
        ingest_fixture(
            &db_path,
            "memory:finance",
            "Quarterly revenue exceeded analyst expectations.",
        );

        let result = search("A furry pet cat", Some(&db_path), 10).unwrap();
        let first_line = result.lines().next().unwrap();
        assert!(first_line.ends_with("urn:mif:memory:cats"));
    }

    #[test]
    fn search_reports_no_matches_on_an_empty_store() {
        warm_embedding_model_cache();
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        mif_store::VectorStore::open(&db_path).unwrap();

        let result = search("anything", Some(&db_path), 10).unwrap();
        assert_eq!(result, "(no matches)");
    }

    #[test]
    fn find_similar_excludes_the_anchor_document_itself() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        ingest_fixture(&db_path, "memory:a", "Cats are small domesticated felines.");
        ingest_fixture(&db_path, "memory:b", "Dogs are loyal domesticated canines.");

        let result = find_similar("urn:mif:memory:a", Some(&db_path), 10).unwrap();
        assert!(!result.contains("memory:a"));
        assert!(result.contains("memory:b"));
    }

    #[test]
    fn find_similar_reports_document_not_found() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        mif_store::VectorStore::open(&db_path).unwrap();

        let error = find_similar("urn:mif:memory:missing", Some(&db_path), 10).unwrap_err();
        let problem = error.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://mif-spec.dev/errors/document-not-found/v1"
        );
        assert_eq!(problem.status, 404);
    }

    #[test]
    fn corpus_stats_reports_count_and_dim() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        assert_eq!(
            corpus_stats(Some(&db_path)).unwrap(),
            format!("count=0 db={}", db_path.display())
        );

        ingest_fixture(&db_path, "memory:one", "Some content.");
        let result = corpus_stats(Some(&db_path)).unwrap();
        assert!(result.contains("count=1"));
        assert!(result.contains("dim=384"));
    }
}
