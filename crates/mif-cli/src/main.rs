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

use std::collections::HashSet;
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
    /// otherwise. Given as `json`, it also makes `search` and `get-passage`
    /// print their results as JSON; without the flag they print text.
    #[arg(long, global = true, value_parser = ["pretty", "json"])]
    format: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a MIF document against the canonical schema. No side
    /// effects: no embedding model load, no vector store write.
    Validate {
        /// Path to the MIF document (markdown with frontmatter, or a
        /// JSON-LD projection) to validate.
        file: PathBuf,
        /// MIF level floor to additionally require (1, 2, or 3). Level 1's
        /// per-document fields are already covered by the canonical schema,
        /// so the default is a plain schema validation. MIF 1.4.0 Level 1
        /// also requires an OKF bundle, which this per-file check does not
        /// cover.
        #[arg(long, default_value_t = 1)]
        level: u8,
    },
    /// Ontology-related operations.
    Ontology {
        #[command(subcommand)]
        command: OntologyCommand,
    },
    /// Lint, validate, prove a lossless round trip, embed, and store MIF
    /// documents. Each document is embedded as a summary passage plus one or
    /// more passages per body section. With one file, a failure is that
    /// file's own error; with several files or a directory, every file is
    /// attempted, each failure is reported on stderr as one plain text line
    /// (`FILE: error: ...`, whatever `--format` says, ahead of the final
    /// error), and the exit code is non-zero if any failed.
    Ingest {
        /// MIF documents (markdown with frontmatter, or JSON-LD
        /// projections) to ingest. A directory stands for every `.md` file
        /// with frontmatter beneath it, skipping hidden directories.
        #[arg(required = true, num_args = 1.., value_name = "FILE")]
        files: Vec<PathBuf>,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`, created (along with its parent directory) if
        /// absent.
        #[arg(long)]
        db_path: Option<PathBuf>,
        /// Skip a document the store already holds with the same content
        /// and current passages.
        #[arg(long)]
        changed_only: bool,
        /// Leave out files and directories beneath a directory argument
        /// that match this pattern. Repeatable. A pattern with no `/`
        /// matches a file or directory name at any depth (`drafts`,
        /// `*.draft.md`); one with a `/` matches the path relative to the
        /// directory argument (`notes/archive/**`). `*` and `?` stay within
        /// one path component and `**` crosses them. A matching directory
        /// is not entered. A file named as an argument itself is always
        /// ingested, even from a hidden or ignored directory; `prune` does
        /// not look there, so its next run removes that document unless
        /// the file is also reachable from the prune root.
        #[arg(long, value_name = "GLOB")]
        ignore: Vec<String>,
    },
    /// Free-text search over previously ingested documents, by meaning and,
    /// by default, by exact terms as well (see `--mode`).
    Search {
        /// The query text to embed and rank stored documents against.
        /// Accepts a value starting with `-` (free-authored prose); the
        /// accepted trade-off is that an unrecognized flag-like token
        /// (e.g. a typo'd `--flag`) is silently accepted as this
        /// positional instead of producing an "unknown argument" error.
        #[arg(allow_hyphen_values = true)]
        query: String,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
        /// Additional `SQLite` vector store database(s) to search alongside
        /// `--db-path` (or its default), merged into one ranked list: by
        /// cosine similarity in `semantic` mode; in `hybrid` mode cosine
        /// similarities are ranked together and each document keeps the
        /// lexical rank it has in its own store. Repeatable (`--extra-db-path a --extra-db-path
        /// b`). A root queried more than once (the same path given twice, or
        /// given as both `--db-path` and `--extra-db-path`) is not
        /// deduplicated, so its rows are counted and ranked once per
        /// occurrence.
        #[arg(long)]
        extra_db_path: Vec<PathBuf>,
        /// Maximum number of ranked results to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Only documents whose id starts with this.
        #[arg(long)]
        id_prefix: Option<String>,
        /// Only documents whose source path, as recorded at ingest, starts
        /// with this. The path is relative to the directory that holds the
        /// store's own directory (for `vault/.mif/vectors.db`, `vault`).
        /// Documents ingested before sources were recorded never match.
        #[arg(long)]
        source_prefix: Option<String>,
        /// Only documents whose MIF namespace starts with this.
        #[arg(long)]
        namespace_prefix: Option<String>,
        /// Only documents whose ontology entity type (`entity.entity_type`)
        /// is exactly this. Repeatable: any of the values given matches.
        /// Documents ingested before types were recorded never match.
        #[arg(long, value_name = "TYPE")]
        entity_type: Vec<String>,
        /// No document whose ontology entity type is exactly this.
        /// Repeatable.
        #[arg(long, value_name = "TYPE")]
        exclude_entity_type: Vec<String>,
        /// Only documents whose concept type (`conceptType`) is exactly
        /// this. Repeatable: any of the values given matches.
        #[arg(long, value_name = "TYPE")]
        concept_type: Vec<String>,
        /// No document whose concept type is exactly this. Repeatable.
        #[arg(long, value_name = "TYPE")]
        exclude_concept_type: Vec<String>,
        /// How to rank. `hybrid` fuses the semantic ranking with a lexical
        /// one over passage text, so an identifier or an exact term is
        /// found by the text that holds it; the score is then a reciprocal
        /// rank fusion value, not a cosine similarity. `semantic` ranks by
        /// cosine similarity alone, as releases before 0.13.0 did.
        #[arg(long, value_parser = ["semantic", "hybrid"], default_value = "hybrid")]
        mode: String,
        /// Append the heading of each document's best-matching passage to
        /// its result line, as `[Heading]`.
        #[arg(long)]
        show_passage: bool,
    },
    /// Print one section of an ingested document as it was written. A
    /// heading that occurs more than once in the document prints each
    /// occurrence. With no `--heading`, prints the document's summary and
    /// then any text before its first heading, which is what a match with
    /// no heading points at.
    GetPassage {
        /// The id of the document (as reported by `ingest` and `search`).
        id: String,
        /// The section's heading, as `search` reports it. Omit for the
        /// document's summary and the text before its first heading.
        #[arg(long)]
        heading: Option<String>,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
        /// Additional `SQLite` vector store database(s) to look in after
        /// `--db-path` (or its default). The document comes from the first
        /// root, in that order, that holds it.
        #[arg(long)]
        extra_db_path: Vec<PathBuf>,
    },
    /// Remove one document's vectors from the vector store.
    Remove {
        /// The id of the document to remove (as reported by `ingest`).
        id: String,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
    },
    /// Remove from the vector store every document that no longer exists
    /// under a directory. A document exists if a `.md` file with
    /// frontmatter, or a JSON-LD file, beneath the directory carries its id.
    /// Hidden directories are not looked in, as `ingest` does not ingest
    /// from them, so a copy left in one does not keep a document. Neither
    /// does a file `ingest` was given by name from a hidden or ignored
    /// directory: that document is removed.
    Prune {
        /// The directory holding the documents to keep.
        #[arg(long)]
        root: PathBuf,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
        /// List what would be removed without removing it.
        #[arg(long)]
        dry_run: bool,
        /// Do not count files and directories that match this pattern.
        /// Repeatable. See `ingest`'s `--ignore` for the pattern rules.
        #[arg(long, value_name = "GLOB")]
        ignore: Vec<String>,
    },
    /// Find previously ingested documents similar to an already-ingested one.
    FindSimilar {
        /// The id of an already-ingested document (as reported by `ingest`).
        id: String,
        /// Path to the `SQLite` vector store database. Defaults to
        /// `.mif/vectors.db`.
        #[arg(long)]
        db_path: Option<PathBuf>,
        /// Additional `SQLite` vector store database(s) to search alongside
        /// `--db-path` (or its default). See `search`'s `--extra-db-path`
        /// for the exact semantics. `id` is looked up across every root (the
        /// first root, in `--db-path` then `--extra-db-path` order, that has
        /// it), and excluded from the merged results wherever it appears.
        #[arg(long)]
        extra_db_path: Vec<PathBuf>,
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
        /// Additional `SQLite` vector store database(s) to summarize
        /// alongside `--db-path` (or its default). See `search`'s
        /// `--extra-db-path` for the exact semantics.
        #[arg(long)]
        extra_db_path: Vec<PathBuf>,
    },
    /// Prove a MIF document's markdown <-> JSON-LD round trip is lossless.
    /// "Lossless" means format fidelity only (no field silently dropped or
    /// changed by the projection) — unlike `validate`, this does not check
    /// schema conformance. Pure: no db, no embedder. Exits non-zero on
    /// drift.
    Roundtrip {
        /// Path to the MIF document (markdown with frontmatter, or a
        /// JSON-LD projection) to check.
        file: PathBuf,
        /// Frontmatter shape to use for standalone JSON-LD input whose
        /// identity fields don't already unambiguously indicate one.
        /// Ignored for markdown input, whose shape is auto-detected.
        /// Defaults to the MIF v1.0 authoring convention.
        #[arg(long, value_parser = ["v1-canonical", "pre-projected"], default_value = "v1-canonical")]
        shape: String,
    },
    /// Project a MIF document to its canonical JSON-LD form, proving the
    /// round trip is lossless (format fidelity, not schema conformance) in
    /// the process. Pure: no db, no embedder.
    EmitJsonld {
        /// Path to the MIF document (markdown with frontmatter, or a
        /// JSON-LD projection) to project.
        file: PathBuf,
        /// Write the JSON-LD projection to this path instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Frontmatter shape to use for standalone JSON-LD input whose
        /// identity fields don't already unambiguously indicate one.
        /// Ignored for markdown input, whose shape is auto-detected.
        /// Defaults to the MIF v1.0 authoring convention.
        #[arg(long, value_parser = ["v1-canonical", "pre-projected"], default_value = "v1-canonical")]
        shape: String,
    },
    /// Project a JSON-LD MIF document to its canonical
    /// markdown-with-frontmatter form, proving the round trip is lossless
    /// in the process. Pure: no db, no embedder.
    EmitMarkdown {
        /// Path to the JSON-LD MIF document to project.
        file: PathBuf,
        /// Write the markdown to this path instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Frontmatter shape to use when the JSON-LD's identity fields
        /// don't already unambiguously indicate one. Defaults to the MIF
        /// v1.0 authoring convention.
        #[arg(long, value_parser = ["v1-canonical", "pre-projected"], default_value = "v1-canonical")]
        shape: String,
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
    /// A JSON-LD projection could not be serialized to JSON text.
    #[error("failed to serialize JSON-LD: {source}")]
    JsonSerialize {
        /// The underlying serialization error.
        #[source]
        source: serde_json::Error,
    },
    /// One or more documents of a multi-document `ingest` failed.
    #[error("{failed} of {total} documents were not ingested")]
    IngestBatch {
        /// How many documents failed.
        failed: usize,
        /// How many documents were attempted.
        total: usize,
    },
    /// `prune` found no document under its root, and so would have removed
    /// every vector in the store.
    #[error("no MIF document found under {root}; refusing to prune every vector in the store")]
    PruneEmptyRoot {
        /// The directory that was scanned.
        root: String,
    },
    /// `get-passage` found the document but nothing stored for it under the
    /// heading asked for.
    #[error(
        "no section {} is stored for document '{id}'",
        heading.as_ref().map_or_else(|| "summary".to_string(), |heading| format!("headed '{heading}'"))
    )]
    SectionNotFound {
        /// The document that was found.
        id: String,
        /// The heading asked for; `None` for the summary.
        heading: Option<String>,
    },
    /// An `--ignore` pattern is not a valid glob.
    #[error("invalid --ignore pattern '{pattern}': {source}")]
    InvalidIgnore {
        /// The pattern as given.
        pattern: String,
        /// Why it could not be read.
        #[source]
        source: globset::Error,
    },
}

impl CliError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::Io { .. } => ProblemMeta {
                slug: "mif-cli-io",
                version: "v1",
                title: "Failed to read an input file",
                status: 500,
                exit_code: 1,
            },
            Self::Json { .. } => ProblemMeta {
                slug: "mif-cli-invalid-json",
                version: "v1",
                title: "Input file is not valid JSON",
                status: 400,
                exit_code: 2,
            },
            Self::DocumentNotFound(_) => ProblemMeta {
                slug: "mif-cli-document-not-found",
                version: "v1",
                title: "No document with the given id has been ingested",
                status: 404,
                exit_code: 3,
            },
            Self::JsonSerialize { .. } => ProblemMeta {
                slug: "mif-cli-json-serialize-failure",
                version: "v1",
                title: "Failed to serialize JSON-LD to text",
                status: 500,
                exit_code: 1,
            },
            Self::IngestBatch { .. } => ProblemMeta {
                slug: "mif-cli-ingest-batch-failed",
                version: "v1",
                title: "Some documents were not ingested",
                status: 422,
                exit_code: 2,
            },
            Self::PruneEmptyRoot { .. } => ProblemMeta {
                slug: "mif-cli-prune-empty-root",
                version: "v1",
                title: "No MIF document found under the prune root",
                status: 400,
                exit_code: 2,
            },
            Self::SectionNotFound { .. } => ProblemMeta {
                slug: "mif-cli-section-not-found",
                version: "v1",
                title: "No such section is stored for the document",
                status: 404,
                exit_code: 3,
            },
            Self::InvalidIgnore { .. } => ProblemMeta {
                slug: "mif-cli-invalid-ignore-pattern",
                version: "v1",
                title: "An --ignore pattern is not a valid glob",
                status: 400,
                exit_code: 2,
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
            Self::IngestBatch { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string())
                .with_suggested_fix(mif_problem::SuggestedFix::new(
                    "Fix each document named on stderr and ingest again; the documents that \
                     succeeded are stored.",
                    mif_problem::Applicability::MaybeIncorrect,
                )),
            Self::PruneEmptyRoot { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string())
                .with_suggested_fix(mif_problem::SuggestedFix::new(
                    "Check `--root` names the directory that holds the ingested documents.",
                    mif_problem::Applicability::MaybeIncorrect,
                )),
            Self::SectionNotFound { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string())
                .with_suggested_fix(mif_problem::SuggestedFix::new(
                    "Use a heading exactly as `mif-cli search --show-passage` reports it. A \
                     document ingested by a release before 0.13.0 has no sections until it is \
                     ingested again.",
                    mif_problem::Applicability::MaybeIncorrect,
                )),
            Self::InvalidIgnore { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string())
                .with_suggested_fix(mif_problem::SuggestedFix::new(
                    "Write the pattern as a glob: `*` and `?` within one path component, `**` \
                     across them, `[abc]` for a character class.",
                    mif_problem::Applicability::MaybeIncorrect,
                )),
            Self::Json { .. } | Self::JsonSerialize { .. } => self
                .meta()
                .into_details(env!("CARGO_PKG_NAME"), self.to_string()),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = OutputFormat::select(cli.format.as_deref(), std::io::stderr().is_terminal());
    let json = cli.format.as_deref() == Some("json");
    match run(&cli.command, json) {
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

/// Runs one subcommand. `json` is whether `--format json` was given, which
/// `search` and `get-passage` print their results by.
fn run(command: &Command, json: bool) -> Result<String, CliError> {
    match command {
        Command::Validate { file, level } => validate(file, *level),
        Command::Ontology { command } => match command {
            OntologyCommand::Resolve { id, ontologies_dir } => resolve(id, ontologies_dir),
        },
        Command::Ingest {
            files,
            db_path,
            changed_only,
            ignore,
        } => ingest_paths(files, db_path.as_deref(), *changed_only, ignore),
        Command::Search {
            query,
            db_path,
            extra_db_path,
            limit,
            id_prefix,
            source_prefix,
            namespace_prefix,
            entity_type,
            exclude_entity_type,
            concept_type,
            exclude_concept_type,
            mode,
            show_passage,
        } => search(
            query,
            db_path.as_deref(),
            extra_db_path,
            &SearchOptions {
                limit: *limit,
                filter: mif_store::SearchFilter {
                    id_prefix: id_prefix.clone(),
                    source_prefix: source_prefix.clone(),
                    namespace_prefix: namespace_prefix.clone(),
                    entity_types: entity_type.clone(),
                    exclude_entity_types: exclude_entity_type.clone(),
                    concept_types: concept_type.clone(),
                    exclude_concept_types: exclude_concept_type.clone(),
                },
                mode: if mode == "semantic" {
                    mif_store::SearchMode::Semantic
                } else {
                    mif_store::SearchMode::Hybrid
                },
                show_passage: *show_passage,
                json,
            },
        ),
        Command::GetPassage {
            id,
            heading,
            db_path,
            extra_db_path,
        } => get_passage(
            id,
            heading.as_deref(),
            db_path.as_deref(),
            extra_db_path,
            json,
        ),
        Command::Remove { id, db_path } => remove(id, db_path.as_deref()),
        Command::Prune {
            root,
            db_path,
            dry_run,
            ignore,
        } => prune(root, db_path.as_deref(), *dry_run, ignore),
        Command::FindSimilar {
            id,
            db_path,
            extra_db_path,
            limit,
        } => find_similar(id, db_path.as_deref(), extra_db_path, *limit),
        Command::CorpusStats {
            db_path,
            extra_db_path,
        } => corpus_stats(db_path.as_deref(), extra_db_path),
        Command::Roundtrip { file, shape } => roundtrip(
            file,
            mif_frontmatter::FrontmatterShape::try_from(shape.as_str())?,
        ),
        Command::EmitJsonld { file, out, shape } => emit_jsonld(
            file,
            out.as_deref(),
            mif_frontmatter::FrontmatterShape::try_from(shape.as_str())?,
        ),
        Command::EmitMarkdown { file, out, shape } => emit_markdown(
            file,
            out.as_deref(),
            mif_frontmatter::FrontmatterShape::try_from(shape.as_str())?,
        ),
    }
}

/// Resolves an optional `--db-path` to the effective vector store path.
fn resolve_db_path(db_path: Option<&Path>) -> PathBuf {
    db_path.map_or_else(|| PathBuf::from(DEFAULT_DB_PATH), Path::to_path_buf)
}

/// Resolves an optional primary `--db-path` plus zero or more
/// `--extra-db-path` values to the full ordered list of vector store roots
/// to query: `--db-path` (defaulting to `.mif/vectors.db` exactly as
/// [`resolve_db_path`] does), followed by every `--extra-db-path` in the
/// order given.
fn resolve_db_paths(db_path: Option<&Path>, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![resolve_db_path(db_path)];
    roots.extend(extra.iter().cloned());
    roots
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

/// Formats a multi-root ranked similarity result list for display, one line
/// per match. Unlike [`format_matches`], each line also shows which root
/// produced the match — with more than one root queried, two matches can
/// share an `id` (the same document copied into multiple roots), so the
/// root is what tells them apart.
fn format_rooted_matches(matches: &[mif_store::RootedMatch]) -> String {
    if matches.is_empty() {
        return "(no matches)".to_string();
    }
    matches
        .iter()
        .map(|m| format!("{:.4}  {}  (root={})", m.score, m.id, m.root.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// How `search` ranks and prints.
#[derive(Debug, Clone, Default)]
struct SearchOptions {
    /// Maximum number of ranked results to return.
    limit: usize,
    /// Which stored documents to rank.
    filter: mif_store::SearchFilter,
    /// How to rank them.
    mode: mif_store::SearchMode,
    /// Whether to append each match's passage heading to its line.
    show_passage: bool,
    /// Whether to print the matches as JSON, with everything known about
    /// each, instead of one text line per match.
    json: bool,
}

/// The ` [Heading]` suffix of a result line, when headings are asked for
/// and the best-matching passage has one.
fn passage_suffix(heading: Option<&str>, show_passage: bool) -> String {
    heading
        .filter(|_| show_passage)
        .map_or_else(String::new, |heading| format!("  [{heading}]"))
}

/// Formats `search` results for display, one line per document, in the
/// same `score  id` form as [`format_matches`].
fn format_search_matches(
    matches: &[mif_store::SearchMatch],
    show_passage: bool,
    places: usize,
) -> String {
    if matches.is_empty() {
        return "(no matches)".to_string();
    }
    matches
        .iter()
        .map(|m| {
            let passage = passage_suffix(m.heading.as_deref(), show_passage);
            format!("{:.places$}  {}{passage}", m.score, m.id)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Formats multi-root `search` results for display, in the same form as
/// [`format_rooted_matches`].
fn format_rooted_search_matches(
    matches: &[mif_store::RootedSearchMatch],
    show_passage: bool,
    places: usize,
) -> String {
    if matches.is_empty() {
        return "(no matches)".to_string();
    }
    matches
        .iter()
        .map(|m| {
            let passage = passage_suffix(m.document.heading.as_deref(), show_passage);
            format!(
                "{:.places$}  {}  (root={}){passage}",
                m.document.score,
                m.document.id,
                m.root.display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How many decimal places a score is printed to. A cosine similarity keeps
/// the four places it has always had. A fusion score is small and falls off
/// slowly with rank, so it needs six for neighbouring results to differ.
const fn score_places(mode: mif_store::SearchMode) -> usize {
    match mode {
        mif_store::SearchMode::Semantic => 4,
        mif_store::SearchMode::Hybrid => 6,
    }
}

/// A span as JSON: its character offsets and lines in the document body.
fn span_json(span: mif_store::PassageSpan) -> serde_json::Value {
    serde_json::json!({
        "start_char": span.start_char,
        "end_char": span.end_char,
        "start_line": span.start_line,
        "end_line": span.end_line,
    })
}

/// One `search` match as JSON: the document, the passage that matched, and
/// the scores. A field with nothing to report is `null`; `root` is present
/// only for a multi-root search.
fn match_json(matched: &mif_store::SearchMatch, root: Option<&Path>) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "id": matched.id,
        "score": matched.score,
        "cosine": matched.cosine,
        "bm25": matched.bm25,
        "path": matched.source,
        "title": matched.title,
        "entity_type": matched.entity_type,
        "concept_type": matched.concept_type,
        "heading": matched.heading,
        "text": matched.text,
        "span": matched.span.map(span_json),
    });
    if let (Some(root), Some(fields)) = (root, entry.as_object_mut()) {
        fields.insert("root".to_string(), root.display().to_string().into());
    }
    entry
}

/// Serializes a result as pretty-printed JSON text.
fn to_json_text(value: &serde_json::Value) -> Result<String, CliError> {
    serde_json::to_string_pretty(value).map_err(|source| CliError::JsonSerialize { source })
}

/// Validates one MIF document: projects markdown-with-frontmatter or
/// JSON-LD input to JSON-LD via [`project_to_jsonld`] (proving the
/// markdown <-> JSON-LD round trip is lossless either way), then
/// schema-checks the result against the canonical schema and the requested
/// `level` floor (1, 2, or 3). Unlike [`ingest_one`], this has no side effects:
/// no embedding model load, no vector store write.
///
/// # Errors
///
/// Returns [`CliError`] if the file cannot be read, is not valid
/// JSON-LD/frontmatter, does not round-trip losslessly, does not conform to
/// the canonical MIF schema, does not satisfy the requested level floor, or
/// `level` is not 1, 2, or 3.
fn validate(file: &Path, level: u8) -> Result<String, CliError> {
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })?;
    let jsonld = project_to_jsonld(
        file,
        &contents,
        mif_frontmatter::FrontmatterShape::V1Canonical,
    )?;
    let level = mif_schema::Level::try_from(level)?;
    mif_schema::validate_level(&jsonld, level)?;
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
/// markdown-with-frontmatter (starts with `---`) or already JSON-LD. A
/// leading UTF-8 BOM (common in files saved by some Windows editors) is
/// stripped first so it doesn't defeat that dispatch.
fn project_to_jsonld(
    path: &Path,
    contents: &str,
    shape: mif_frontmatter::FrontmatterShape,
) -> Result<serde_json::Value, CliError> {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
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

/// What an ingest needs once a document has validated: the embedding model
/// and the vector store, each set up on first use. A document that fails
/// validation therefore costs neither, and a run over many documents loads
/// the model once.
struct IngestContext {
    db_path: PathBuf,
    embedder: Option<mif_embed::Embedder>,
    store: Option<mif_store::VectorStore>,
}

impl IngestContext {
    fn new(db_path: Option<&Path>) -> Self {
        Self {
            db_path: resolve_db_path(db_path),
            embedder: None,
            store: None,
        }
    }

    fn embedder(&mut self) -> Result<&mif_embed::Embedder, CliError> {
        let embedder = match self.embedder.take() {
            Some(embedder) => embedder,
            None => mif_embed::Embedder::load()?,
        };
        Ok(self.embedder.insert(embedder))
    }

    fn store(&mut self) -> Result<&mif_store::VectorStore, CliError> {
        let store = if let Some(store) = self.store.take() {
            store
        } else {
            if let Some(parent) = self.db_path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent).map_err(|source| CliError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
            mif_store::VectorStore::open(&self.db_path)?
        };
        Ok(self.store.insert(store))
    }
}

/// Ingests every document named by `paths` (see `ingest`'s `FILE`
/// argument).
///
/// One file that is not a directory is ingested by [`ingest_one`], and its
/// failure is the command's failure. Anything else is a batch: each result line is printed as it is produced, each
/// failure is reported on stderr, and the batch goes on to the next file.
///
/// # Errors
///
/// For one file, that file's own [`CliError`]. For a batch,
/// [`CliError::Io`] if a directory cannot be read, or
/// [`CliError::IngestBatch`] if any document failed.
fn ingest_paths(
    paths: &[PathBuf],
    db_path: Option<&Path>,
    changed_only: bool,
    ignore: &[String],
) -> Result<String, CliError> {
    let ignore = Ignore::new(ignore)?;
    let mut context = IngestContext::new(db_path);
    if let [file] = paths
        && !file.is_dir()
    {
        return ingest_one(file, &mut context, changed_only);
    }

    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found = Vec::new();
            walk(path, path, &ignore, &mut found)?;
            files.extend(found.into_iter().filter(|file| is_markdown_document(file)));
        } else {
            files.push(path.clone());
        }
    }

    let mut failed = 0;
    for file in &files {
        match ingest_one(file, &mut context, changed_only) {
            Ok(line) => println!("{line}"),
            Err(error) => {
                failed += 1;
                eprintln!("{}: error: {error}", file.display());
            },
        }
    }
    if failed > 0 {
        return Err(CliError::IngestBatch {
            failed,
            total: files.len(),
        });
    }
    Ok(format!(
        "ingested {} documents (db={})",
        files.len(),
        context.db_path.display()
    ))
}

/// Runs the full ingestion pipeline for one MIF document: validate (lint),
/// prove a lossless round trip, embed its passages, and store them, with
/// the model and store taken from `context`. With `changed_only`, a
/// document the store already holds with the same content and current
/// passages is left alone.
///
/// # Errors
///
/// Returns [`CliError`] if the file cannot be read, does not conform to the
/// canonical MIF schema, does not round-trip losslessly, the embedding model
/// cannot be loaded or run, or the vector store cannot be opened or written.
fn ingest_one(
    file: &Path,
    context: &mut IngestContext,
    changed_only: bool,
) -> Result<String, CliError> {
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
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
    let hash = content_hash(&contents);
    let db = context.db_path.display().to_string();
    let source = mif_store::document_source(&context.db_path, file);
    if changed_only
        && context.db_path.exists()
        && context.store()?.is_current(&id, &hash, Some(&source))?
    {
        return Ok(format!(
            "{}: unchanged stored=false (id={id}, db={db})",
            file.display()
        ));
    }

    let embedder = context.embedder()?;
    let stored = embedder.embed_document(&jsonld)?;
    let model = embedder.model_id();
    let model_version = embedder
        .model_revision()
        .map_or_else(String::new, |revision| format!(" model_version={revision}"));

    let summary = stored.first().map_or(&[][..], |p| p.vector.as_slice());
    let passages: Vec<mif_store::PassageVector<'_>> = stored
        .iter()
        .map(|p| mif_store::PassageVector {
            heading: p.heading.as_deref(),
            vector: &p.vector,
            text: Some(&p.text),
            span: p.span.map(store_span),
            section: p.section,
        })
        .collect();
    let written = mif_embed::passages::sections(&jsonld);
    let sections: Vec<mif_store::SectionText<'_>> = written
        .iter()
        .map(|section| mif_store::SectionText {
            heading: section.heading.as_deref(),
            text: &section.text,
            span: store_span(section.span),
        })
        .collect();
    let updated_at = chrono::Utc::now().to_rfc3339();
    context
        .store()?
        .upsert_document(&mif_store::DocumentVectors {
            id: &id,
            vector: summary,
            passages: &passages,
            content_hash: &hash,
            updated_at: &updated_at,
            source: Some(&source),
            namespace: text_at(&jsonld, "namespace"),
            title: text_at(&jsonld, "title"),
            entity_type: jsonld
                .get("entity")
                .and_then(|entity| text_at(entity, "entity_type")),
            // `memoryType` is the deprecated name of `conceptType`.
            concept_type: text_at(&jsonld, "conceptType")
                .or_else(|| text_at(&jsonld, "memoryType")),
            sections: &sections,
        })?;

    Ok(format!(
        "{}: lint=ok validate=ok roundtrip=lossless embedding_dim={} passages={} model={model}{model_version} normalized={} stored=true (id={id}, db={db})",
        file.display(),
        summary.len(),
        passages.len(),
        mif_embed::NORMALIZED,
    ))
}

/// A string field of a JSON object.
fn text_at<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}

/// A span from the passage builder as the store records it.
const fn store_span(span: mif_embed::passages::Span) -> mif_store::PassageSpan {
    mif_store::PassageSpan {
        start_char: span.start_char,
        end_char: span.end_char,
        start_line: span.start_line,
        end_line: span.end_line,
    }
}

/// The `--ignore` patterns of an `ingest` or a `prune`.
///
/// A pattern with no `/` is matched against a file or directory name, so it
/// applies at any depth; one with a `/` is matched against the path
/// relative to the directory being walked. `*` and `?` stay within one
/// path component, and `**` crosses them.
struct Ignore {
    names: globset::GlobSet,
    paths: globset::GlobSet,
}

impl Ignore {
    fn new(patterns: &[String]) -> Result<Self, CliError> {
        let mut names = globset::GlobSetBuilder::new();
        let mut paths = globset::GlobSetBuilder::new();
        for pattern in patterns {
            let invalid = |source| CliError::InvalidIgnore {
                pattern: pattern.clone(),
                source,
            };
            // `drafts/` names the directory `drafts`; `/notes` is anchored
            // to the walked directory, which every path pattern already is.
            let trimmed = pattern.trim_end_matches('/');
            let anchored = trimmed.contains('/');
            let glob = globset::GlobBuilder::new(trimmed.trim_start_matches('/'))
                .literal_separator(true)
                .build()
                .map_err(invalid)?;
            if anchored {
                paths.add(glob);
            } else {
                names.add(glob);
            }
        }
        let invalid = |source| CliError::InvalidIgnore {
            pattern: patterns.join(" "),
            source,
        };
        Ok(Self {
            names: names.build().map_err(invalid)?,
            paths: paths.build().map_err(invalid)?,
        })
    }

    /// Whether the entry at `path`, beneath the walked directory `root`, is
    /// left out.
    fn matches(&self, root: &Path, path: &Path) -> bool {
        path.file_name()
            .is_some_and(|name| self.names.is_match(name))
            || path
                .strip_prefix(root)
                .is_ok_and(|relative| self.paths.is_match(relative))
    }
}

/// Collects every file beneath `dir`, in path order, including files
/// reached through a symbolic link, leaving out whatever `ignore` matches
/// (by its path relative to `root`, the directory the walk started at).
/// `.git`, `node_modules`, and any directory whose name starts with a dot
/// are never entered. Symbolic links to directories are not followed.
fn walk(dir: &Path, root: &Path, ignore: &Ignore, out: &mut Vec<PathBuf>) -> Result<(), CliError> {
    let io_error = |source| CliError::Io {
        path: dir.display().to_string(),
        source,
    };
    let mut entries = std::fs::read_dir(dir)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if ignore.matches(root, &path) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let file_type = entry.file_type().map_err(io_error)?;
        // A link to a file is that file; a link to a directory is not
        // entered, since it may lead back into the tree being walked.
        if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
            out.push(path);
        } else if file_type.is_dir() && name != "node_modules" && !name.starts_with('.') {
            walk(&path, root, ignore, out)?;
        }
    }
    Ok(())
}

/// Whether `file` is a `.md` file that opens with frontmatter. A file that
/// cannot be read is not one.
fn is_markdown_document(file: &Path) -> bool {
    file.extension().is_some_and(|ext| ext == "md")
        && std::fs::read_to_string(file).is_ok_and(|contents| {
            contents
                .strip_prefix('\u{feff}')
                .unwrap_or(&contents)
                .starts_with("---")
        })
}

/// The id `ingest` stores a document under, read from the file without
/// validating it: `None` for a file that is not a MIF document.
///
/// # Errors
///
/// Returns [`CliError`] if the file cannot be read, or opens with
/// frontmatter that cannot be parsed. Such a file may be a document whose
/// id is simply unreadable right now, so it is an error rather than a
/// reason to treat the document as gone.
fn document_id(file: &Path) -> Result<Option<String>, CliError> {
    let extension = file.extension().and_then(std::ffi::OsStr::to_str);
    if !matches!(extension, Some("md" | "json" | "jsonld")) {
        return Ok(None);
    }
    if extension != Some("md") {
        // Any JSON file may sit under the root; only one that reads and
        // parses as an object with an `@id` is a document.
        return Ok(std::fs::read_to_string(file)
            .ok()
            .and_then(|contents| {
                let contents = contents.strip_prefix('\u{feff}').unwrap_or(&contents);
                serde_json::from_str::<serde_json::Value>(contents).ok()
            })
            .and_then(|value| value.get("@id")?.as_str().map(ToString::to_string)));
    }
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })?;
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(&contents);
    match mif_frontmatter::parse_markdown(contents) {
        Ok((frontmatter, _body)) => {
            let literal = frontmatter.get("@id").and_then(|id| id.as_str());
            let short = frontmatter.get("id").and_then(|id| id.as_str());
            Ok(literal
                .map(ToString::to_string)
                .or_else(|| short.map(|id| format!("urn:mif:{id}"))))
        },
        Err(mif_frontmatter::FrontmatterError::MissingFrontmatter) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Removes one document's vectors from the vector store.
///
/// # Errors
///
/// Returns [`CliError`] if the vector store cannot be opened or written.
fn remove(id: &str, db_path: Option<&Path>) -> Result<String, CliError> {
    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let outcome = if store.remove(id)? {
        "removed"
    } else {
        "not stored"
    };
    Ok(format!("{id}: {outcome} (db={})", db_path.display()))
}

/// Removes from the vector store every document whose id is carried by no
/// file under `root`, or with `dry_run` only lists them.
///
/// # Errors
///
/// Returns [`CliError::PruneEmptyRoot`] if no document is found under
/// `root`, or [`CliError`] if `root` cannot be read, a file's frontmatter
/// cannot be parsed, or the vector store cannot be opened or written.
fn prune(
    root: &Path,
    db_path: Option<&Path>,
    dry_run: bool,
    ignore: &[String],
) -> Result<String, CliError> {
    let mut files = Vec::new();
    walk(root, root, &Ignore::new(ignore)?, &mut files)?;
    let mut keep = HashSet::new();
    for file in &files {
        keep.extend(document_id(file)?);
    }
    if keep.is_empty() {
        return Err(CliError::PruneEmptyRoot {
            root: root.display().to_string(),
        });
    }

    let db_path = resolve_db_path(db_path);
    let store = mif_store::VectorStore::open(&db_path)?;
    let (removed, verb) = if dry_run {
        let stale = store.ids()?;
        let stale = stale.into_iter().filter(|id| !keep.contains(id)).collect();
        (stale, "would prune")
    } else {
        (store.prune(&keep)?, "pruned")
    };
    let mut lines: Vec<String> = removed.iter().map(|id| format!("{verb} {id}")).collect();
    lines.push(format!(
        "{verb} {} documents, {} found under {} (db={})",
        removed.len(),
        keep.len(),
        root.display(),
        db_path.display()
    ));
    Ok(lines.join("\n"))
}

/// Embeds `query` and ranks previously ingested documents by
/// `options.mode`: the cosine similarity of each document's best-matching
/// passage, or that ranking fused with a lexical one over passage text (a
/// document ingested before passages existed is ranked by its one document
/// vector).
///
/// When `extra_db_path` is empty, this queries only the resolved `db_path`
/// root, identically to single-root usage before multi-root support existed.
/// Otherwise it queries `db_path` (or its default) together with every
/// `extra_db_path` root, merged into one ranked list by
/// [`mif_store::multi_root_query`].
///
/// # Errors
///
/// Returns [`CliError`] if the embedding model cannot be loaded or run, or
/// any queried vector store root cannot be opened or queried.
fn search(
    query: &str,
    db_path: Option<&Path>,
    extra_db_path: &[PathBuf],
    options: &SearchOptions,
) -> Result<String, CliError> {
    let embedder = mif_embed::Embedder::load()?;
    let vector = embedder.embed(query)?;
    let search = mif_store::SearchQuery {
        vector: &vector,
        text: query,
        mode: options.mode,
        limit: options.limit,
        filter: &options.filter,
    };

    if extra_db_path.is_empty() {
        let db_path = resolve_db_path(db_path);
        let store = mif_store::VectorStore::open(&db_path)?;
        let matches = store.query(&search)?;
        if options.json {
            let matches: Vec<_> = matches.iter().map(|m| match_json(m, None)).collect();
            return to_json_text(&serde_json::json!({ "matches": matches }));
        }
        return Ok(format_search_matches(
            &matches,
            options.show_passage,
            score_places(options.mode),
        ));
    }

    let roots = resolve_db_paths(db_path, extra_db_path);
    let matches = mif_store::multi_root_query(&roots, &search)?;
    if options.json {
        let matches: Vec<_> = matches
            .iter()
            .map(|m| match_json(&m.document, Some(&m.root)))
            .collect();
        return to_json_text(&serde_json::json!({ "matches": matches }));
    }
    Ok(format_rooted_search_matches(
        &matches,
        options.show_passage,
        score_places(options.mode),
    ))
}

/// Prints the sections of an ingested document headed `heading`, as they
/// were written, or with no `heading` the document's summary and the text
/// before its first heading.
///
/// With `extra_db_path`, the document comes from the first root (`db_path`,
/// then each `extra_db_path` in order) that holds it.
///
/// # Errors
///
/// Returns [`CliError::DocumentNotFound`] if `id` is in no queried root,
/// [`CliError::SectionNotFound`] if the document has nothing stored under
/// `heading`, or [`CliError`] if a vector store root cannot be opened or
/// queried.
fn get_passage(
    id: &str,
    heading: Option<&str>,
    db_path: Option<&Path>,
    extra_db_path: &[PathBuf],
    json: bool,
) -> Result<String, CliError> {
    // One root is asked directly, so its failures are its own and not a
    // multi-root one.
    let found = if extra_db_path.is_empty() {
        let root = resolve_db_path(db_path);
        mif_store::VectorStore::open(&root)?
            .sections(id, heading)?
            .map(|sections| (root, sections))
    } else {
        let roots = resolve_db_paths(db_path, extra_db_path);
        mif_store::multi_root_sections(&roots, id, heading)?
    };
    let (root, sections) = found.ok_or_else(|| CliError::DocumentNotFound(id.to_string()))?;
    if sections.is_empty() {
        return Err(CliError::SectionNotFound {
            id: id.to_string(),
            heading: heading.map(ToString::to_string),
        });
    }
    if !json {
        return Ok(sections
            .iter()
            .map(|section| section.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"));
    }
    let sections: Vec<_> = sections
        .iter()
        .map(|section| {
            serde_json::json!({
                "heading": section.heading,
                "text": section.text,
                "span": section.span.map(span_json),
            })
        })
        .collect();
    let mut result = serde_json::json!({ "id": id, "sections": sections });
    if let (false, Some(fields)) = (extra_db_path.is_empty(), result.as_object_mut()) {
        fields.insert("root".to_string(), root.display().to_string().into());
    }
    to_json_text(&result)
}

/// Finds documents similar to an already-ingested one, identified by `id`.
///
/// Follows the same single-root-unless-`extra_db_path`-given behavior as
/// [`search`]. With more than one root, `id` is looked up across every root
/// (the first, in `db_path` then `extra_db_path` order, that has it) and
/// excluded from the merged results wherever it appears across every root,
/// not just the one it was found in — see
/// [`mif_store::RootedMatch`]'s doc comment for why that distinction
/// matters once roots can carry colliding ids.
///
/// # Errors
///
/// Returns [`CliError::DocumentNotFound`] if `id` has never been ingested
/// into any queried root, or [`CliError`] if any queried vector store root
/// cannot be opened or queried.
fn find_similar(
    id: &str,
    db_path: Option<&Path>,
    extra_db_path: &[PathBuf],
    limit: usize,
) -> Result<String, CliError> {
    // Request extra matches so excluding the anchor document itself still
    // leaves up to `limit` genuinely-similar results. `saturating_add` avoids
    // an overflow panic (debug builds) / silent wraparound (release builds)
    // if a caller passes `limit = usize::MAX`.
    //
    // Single-root: the anchor id is unique within one `VectorStore`, so it
    // occupies exactly one slot in the fetched window — `limit + 1` is
    // always enough.
    if extra_db_path.is_empty() {
        let over_fetch = limit.saturating_add(1);
        let db_path = resolve_db_path(db_path);
        let store = mif_store::VectorStore::open(&db_path)?;
        let anchor = store
            .get(id)?
            .ok_or_else(|| CliError::DocumentNotFound(id.to_string()))?;
        let matches: Vec<_> = store
            .top_k_similar(&anchor.vector, over_fetch)?
            .into_iter()
            .filter(|m| m.id != id)
            .take(limit)
            .collect();
        return Ok(format_matches(&matches));
    }

    // Multi-root: the anchor id can independently exist in every queried
    // root (see `RootedMatch`'s doc comment), so it can occupy up to
    // `roots.len()` slots in the merged window, not just one. Sizing the
    // over-fetch as `limit + 1` here would let those extra anchor copies
    // crowd out genuinely-better matches before the exclusion filter below
    // ever runs, silently returning fewer than `limit` results even when
    // enough real matches exist.
    let roots = resolve_db_paths(db_path, extra_db_path);
    let over_fetch = limit.saturating_add(roots.len());
    let (_root, anchor) = mif_store::multi_root_get(&roots, id)?
        .ok_or_else(|| CliError::DocumentNotFound(id.to_string()))?;
    let matches: Vec<_> = mif_store::multi_root_top_k_similar(&roots, &anchor.vector, over_fetch)?
        .into_iter()
        .filter(|m| m.id != id)
        .take(limit)
        .collect();
    Ok(format_rooted_matches(&matches))
}

/// Summarizes the vector store's contents.
///
/// A single root also reports how many passages it holds and how many of
/// its documents have none (documents not ingested since passages existed).
///
/// Follows the same single-root-unless-`extra_db_path`-given behavior as
/// [`search`]. With more than one root, the output leads with the summed
/// `total_count` across every root, followed by one line per root showing
/// that root's own count and (if non-empty) representative dimensionality.
///
/// # Errors
///
/// Returns [`CliError`] if any queried vector store root cannot be opened or
/// queried.
fn corpus_stats(db_path: Option<&Path>, extra_db_path: &[PathBuf]) -> Result<String, CliError> {
    if extra_db_path.is_empty() {
        let db_path = resolve_db_path(db_path);
        let store = mif_store::VectorStore::open(&db_path)?;
        let stats = store.stats()?;
        let passages = store.passage_stats()?;
        return Ok(stats.dim.map_or_else(
            || format!("count=0 db={}", db_path.display()),
            |dim| {
                format!(
                    "count={} dim={dim} db={} passages={} documents_without_passages={} \
                     documents_without_text={}",
                    stats.count,
                    db_path.display(),
                    passages.passage_count,
                    passages.documents_without_passages,
                    passages.documents_without_text
                )
            },
        ));
    }

    let roots = resolve_db_paths(db_path, extra_db_path);
    let multi = mif_store::multi_root_stats(&roots)?;
    let mut lines = vec![format!("total_count={}", multi.total_count)];
    lines.extend(multi.per_root.iter().map(|(root, stats)| {
        stats.dim.map_or_else(
            || format!("  count=0 db={}", root.display()),
            |dim| format!("  count={} dim={dim} db={}", stats.count, root.display()),
        )
    }));
    Ok(lines.join("\n"))
}

/// Proves a MIF document's markdown <-> JSON-LD round trip is lossless, by
/// running [`project_to_jsonld`]'s existing round-trip proof and discarding
/// the projected result. `shape` only affects standalone JSON-LD input (see
/// [`mif_frontmatter::FrontmatterShape`]); markdown input's shape is
/// auto-detected and ignores it. Pure: no db, no embedder, deterministic
/// output.
///
/// # Errors
///
/// Returns [`CliError`] if the file cannot be read, is not valid
/// JSON-LD/frontmatter, or does not round-trip losslessly.
fn roundtrip(file: &Path, shape: mif_frontmatter::FrontmatterShape) -> Result<String, CliError> {
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })?;
    project_to_jsonld(file, &contents, shape)?;
    Ok(format!("{}: roundtrip lossless", file.display()))
}

/// Projects a MIF document (markdown-with-frontmatter or already JSON-LD)
/// to its canonical JSON-LD form via [`project_to_jsonld`] (proving the
/// round trip is lossless in the process), then prints or writes it.
/// `shape` only affects standalone JSON-LD input; markdown input's shape is
/// auto-detected and ignores it. Pure: no db, no embedder.
///
/// # Errors
///
/// Returns [`CliError`] if the file cannot be read, is not valid
/// JSON-LD/frontmatter, does not round-trip losslessly, the projection
/// cannot be serialized, or (when `out` is given) the output path cannot be
/// written.
fn emit_jsonld(
    file: &Path,
    out: Option<&Path>,
    shape: mif_frontmatter::FrontmatterShape,
) -> Result<String, CliError> {
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })?;
    let jsonld = project_to_jsonld(file, &contents, shape)?;
    let pretty = serde_json::to_string_pretty(&jsonld)
        .map_err(|source| CliError::JsonSerialize { source })?;
    if let Some(out) = out {
        std::fs::write(out, format!("{pretty}\n")).map_err(|source| CliError::Io {
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
/// the process, then prints or writes it. Pure: no db, no embedder.
///
/// # Errors
///
/// Returns [`CliError`] if the file cannot be read, is not valid JSON, does
/// not round-trip losslessly, or (when `out` is given) the output path
/// cannot be written.
fn emit_markdown(
    file: &Path,
    out: Option<&Path>,
    shape: mif_frontmatter::FrontmatterShape,
) -> Result<String, CliError> {
    let contents = std::fs::read_to_string(file).map_err(|source| CliError::Io {
        path: file.display().to_string(),
        source,
    })?;
    // Strip a leading UTF-8 BOM (common in files saved by some Windows
    // editors), matching `project_to_jsonld`'s handling of the same case.
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(&contents);
    let jsonld: serde_json::Value =
        serde_json::from_str(contents).map_err(|source| CliError::Json {
            path: file.display().to_string(),
            source,
        })?;
    let (frontmatter, body) = mif_frontmatter::jsonld_roundtrip_lossless(&jsonld, shape)?;
    let markdown = mif_frontmatter::serialize_markdown(&frontmatter, &body)?;
    if let Some(out) = out {
        std::fs::write(out, &markdown).map_err(|source| CliError::Io {
            path: out.display().to_string(),
            source,
        })?;
        Ok(format!(
            "{}: wrote markdown to {}",
            file.display(),
            out.display()
        ))
    } else {
        Ok(markdown.trim_end_matches('\n').to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Runs a subcommand the way a call without `--format json` does.
    fn run_text(command: &Command) -> Result<String, CliError> {
        run(command, false)
    }

    use mif_problem::{OutputFormat, ToProblem};

    use mif_frontmatter::FrontmatterShape;

    use super::{
        CliError, Command, IngestContext, OntologyCommand, SearchOptions, corpus_stats,
        emit_jsonld, emit_markdown, find_similar, ingest_one, ingest_paths, passage_suffix, prune,
        remove, resolve, roundtrip, run, search, validate,
    };

    /// Ingests one document, as `mif-cli ingest FILE` does.
    fn ingest(file: &Path, db_path: Option<&Path>) -> Result<String, CliError> {
        ingest_one(file, &mut IngestContext::new(db_path), false)
    }

    /// Default search options with a result limit.
    fn top(limit: usize) -> SearchOptions {
        SearchOptions {
            limit,
            ..SearchOptions::default()
        }
    }

    /// A conformant markdown document with a title, a definition, and two
    /// body sections.
    fn sectioned_document(name: &str, title: &str, scope: &str, owner: &str) -> String {
        format!(
            "---\nid: {}\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\nnamespace: records/{name}\n\
             title: {title}\n---\n\n# {title}\n\n## Scope\n\n{scope}\n\n## Owner\n\n{owner}\n\n\
             ## Relationships\n\n- relates-to [Other](other.md)\n",
            mif_core::concept_uuid(name)
        )
    }

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
                "@id": "urn:mif:89886348-4773-5732-88b5-84366da76468",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        assert_eq!(
            validate(file.path(), 1).unwrap(),
            format!("{}: valid", file.path().display())
        );
    }

    #[test]
    fn validate_accepts_a_conformant_markdown_document() {
        // Regression test for mif-rs#39: `validate` must accept
        // markdown-with-frontmatter directly. `validate`'s own code path
        // never references `mif_embed`/`mif_store`, so the absence of
        // side effects (unlike `ingest`) is a structural property, not
        // one this test runs an assertion against.
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        assert_eq!(
            validate(file.path(), 1).unwrap(),
            format!("{}: valid", file.path().display())
        );
    }

    #[test]
    fn validate_accepts_a_markdown_document_with_a_leading_byte_order_mark() {
        // Regression test: a leading UTF-8 BOM (common in files saved by
        // some Windows editors) must not defeat project_to_jsonld's
        // markdown-vs-JSON-LD dispatch.
        let file = write_temp_file(&format!("\u{feff}{VALID_MARKDOWN_FIXTURE}"));
        assert_eq!(
            validate(file.path(), 1).unwrap(),
            format!("{}: valid", file.path().display())
        );
    }

    #[test]
    fn validate_rejects_a_non_conformant_document() {
        let file = write_temp_file(r#"{"content": "missing required fields"}"#);
        let error = validate(file.path(), 1).unwrap_err();
        assert!(error.to_string().contains("failed schema validation"));
    }

    #[test]
    fn validate_reports_invalid_json() {
        let file = write_temp_file("not json");
        let error = validate(file.path(), 1).unwrap_err();
        assert!(error.to_string().contains("failed to parse"));
    }

    #[test]
    fn validate_reports_missing_file() {
        let missing = std::path::Path::new("/nonexistent/mif-cli-test-fixture.json");
        let error = validate(missing, 1).unwrap_err();
        assert!(error.to_string().contains("failed to read"));
    }

    #[test]
    fn validate_l2_rejects_a_document_missing_the_l2_floor_fields() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:89886348-4773-5732-88b5-84366da76468",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let error = validate(file.path(), 2).unwrap_err();
        assert!(error.to_string().contains("L2 level floor"));
        assert!(error.to_string().contains("namespace"));
    }

    #[test]
    fn validate_l2_accepts_a_document_with_the_l2_floor_fields() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:89886348-4773-5732-88b5-84366da76468",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z",
                "namespace": "test",
                "modified": "2026-07-02T00:00:00Z",
                "temporal": {}
            }"#,
        );
        assert_eq!(
            validate(file.path(), 2).unwrap(),
            format!("{}: valid", file.path().display())
        );
    }

    #[test]
    fn validate_rejects_an_out_of_range_level() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let error = validate(file.path(), 9).unwrap_err();
        assert!(error.to_string().contains("unsupported MIF level"));
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
        let error = validate(file.path(), 1).unwrap_err();
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
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-document/v1"
        );
    }

    #[test]
    fn missing_file_io_error_classifies_as_a_404_maybe_incorrect_problem() {
        let error =
            validate(std::path::Path::new("/nonexistent/mif-cli-fixture.json"), 1).unwrap_err();
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
        let error = validate(dir.path(), 1).unwrap_err();
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
            "https://modeled-information-format.github.io/mif-rs/references/errors/ontology-not-found/v1"
        );
        assert_eq!(problem.status, 404);
    }

    #[test]
    fn pretty_render_matches_error_prefixed_display() {
        let error =
            validate(std::path::Path::new("/nonexistent/mif-cli-fixture.json"), 1).unwrap_err();
        assert_eq!(
            error.render(OutputFormat::Pretty),
            format!("Error: {error}")
        );
    }

    const VALID_MARKDOWN_FIXTURE: &str = "---
id: 89886348-4773-5732-88b5-84366da76468
type: semantic
created: 2026-07-02T00:00:00Z
---

Test content.
";

    // Triggers a genuine RoundTripDrift: a non-string frontmatter key
    // (`123: ...`) is silently dropped by `md_to_jsonld`'s generic
    // pass-through loop, so the recovered markdown differs from the
    // original. See mif-frontmatter's own
    // `non_string_top_level_frontmatter_key_is_dropped_and_causes_roundtrip_drift`.
    const DRIFTING_MARKDOWN_FIXTURE: &str = "---\nid: b9ec008c-8581-5ffd-b826-fc0ae7569661\ntype: semantic\n123: orphaned-value\n---\n\nBody.\n";

    #[test]
    fn roundtrip_accepts_a_conformant_markdown_document() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        assert_eq!(
            roundtrip(file.path(), FrontmatterShape::V1Canonical).unwrap(),
            format!("{}: roundtrip lossless", file.path().display())
        );
    }

    #[test]
    fn roundtrip_accepts_a_conformant_jsonld_document() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:89886348-4773-5732-88b5-84366da76468",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        assert_eq!(
            roundtrip(file.path(), FrontmatterShape::V1Canonical).unwrap(),
            format!("{}: roundtrip lossless", file.path().display())
        );
    }

    #[test]
    fn roundtrip_rejects_a_drifting_document() {
        let file = write_temp_file(DRIFTING_MARKDOWN_FIXTURE);
        let error = roundtrip(file.path(), FrontmatterShape::V1Canonical).unwrap_err();
        assert!(matches!(error, CliError::Frontmatter(_)));
    }

    #[test]
    fn roundtrip_rejects_json_ld_input_whose_fields_do_not_survive_the_trip() {
        // Regression: a prior version of `project_to_jsonld` only proved
        // the markdown DERIVED from JSON-LD input was stable under a
        // further cycle, never that the derived markdown (and its
        // reconstructed JSON-LD) actually matched the ORIGINAL input. A
        // `timestamp` field with no backing `created`/`modified` to
        // regenerate it from is silently dropped by `jsonld_to_md` (it is
        // documented as always a derived-only mirror) — real data loss
        // that must now surface as drift, not a false "lossless" report.
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:86444d37-f6ca-5b8e-a594-378fcb9d3e7e",
                "conceptType": "semantic",
                "content": "Test content.",
                "timestamp": "2026-01-01T00:00:00Z"
            }"#,
        );
        let error = roundtrip(file.path(), FrontmatterShape::V1Canonical).unwrap_err();
        assert!(matches!(error, CliError::Frontmatter(_)));
    }

    #[test]
    fn roundtrip_accepts_json_ld_input_whose_timestamp_is_consistently_derived() {
        // The non-regression counterpart: a schema-valid document (with
        // `created`, whose `timestamp` is exactly what `md_to_jsonld` would
        // (re)compute from it) must still report lossless — the stricter
        // check above must not produce false positives on real documents.
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:33be700f-6122-5746-a35e-59205992dbf9",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-01-01T00:00:00Z",
                "timestamp": "2026-01-01T00:00:00Z"
            }"#,
        );
        assert_eq!(
            roundtrip(file.path(), FrontmatterShape::V1Canonical).unwrap(),
            format!("{}: roundtrip lossless", file.path().display())
        );
    }

    #[test]
    fn emit_jsonld_rejects_json_ld_input_whose_fields_do_not_survive_the_trip() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:eee2ea61-690e-51ca-924e-868bbf304c12",
                "conceptType": "semantic",
                "content": "Test content.",
                "timestamp": "2026-01-01T00:00:00Z"
            }"#,
        );
        let error = emit_jsonld(file.path(), None, FrontmatterShape::V1Canonical).unwrap_err();
        assert!(matches!(error, CliError::Frontmatter(_)));
    }

    #[test]
    fn emit_jsonld_prints_the_projection_to_stdout_by_default() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let output = emit_jsonld(file.path(), None, FrontmatterShape::V1Canonical).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["@id"], "urn:mif:89886348-4773-5732-88b5-84366da76468");
        assert_eq!(value["conceptType"], "semantic");
    }

    #[test]
    fn emit_jsonld_writes_to_the_out_path_when_given() {
        let file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("out.json");
        let message =
            emit_jsonld(file.path(), Some(&out_path), FrontmatterShape::V1Canonical).unwrap();
        assert!(message.contains("wrote JSON-LD to"));
        let written = fs::read_to_string(&out_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value["@id"], "urn:mif:89886348-4773-5732-88b5-84366da76468");
    }

    #[test]
    fn emit_jsonld_rejects_a_drifting_document() {
        let file = write_temp_file(DRIFTING_MARKDOWN_FIXTURE);
        let error = emit_jsonld(file.path(), None, FrontmatterShape::V1Canonical).unwrap_err();
        assert!(matches!(error, CliError::Frontmatter(_)));
    }

    #[test]
    fn emit_markdown_prints_the_projection_to_stdout_by_default() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:89886348-4773-5732-88b5-84366da76468",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let output = emit_markdown(
            file.path(),
            None,
            mif_frontmatter::FrontmatterShape::V1Canonical,
        )
        .unwrap();
        assert!(output.starts_with("---\n"));
        assert!(output.contains("Test content."));
    }

    #[test]
    fn emit_markdown_accepts_json_ld_with_a_leading_byte_order_mark() {
        // Regression: emit_markdown parsed JSON directly, unlike
        // project_to_jsonld (used by validate/ingest/roundtrip/emit-jsonld),
        // which strips a leading UTF-8 BOM first. A BOM-prefixed file that
        // every other command accepted failed here.
        let mut contents = "\u{feff}".to_string();
        contents.push_str(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:24b316e1-6c73-5df2-af96-15bf0c95da4b",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let file = write_temp_file(&contents);
        let output = emit_markdown(
            file.path(),
            None,
            mif_frontmatter::FrontmatterShape::V1Canonical,
        )
        .unwrap();
        assert!(output.starts_with("---\n"));
    }

    #[test]
    fn emit_markdown_writes_to_the_out_path_when_given() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:89886348-4773-5732-88b5-84366da76468",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let out_dir = tempfile::tempdir().unwrap();
        let out_path = out_dir.path().join("out.md");
        let message = emit_markdown(
            file.path(),
            Some(&out_path),
            mif_frontmatter::FrontmatterShape::V1Canonical,
        )
        .unwrap();
        assert!(message.contains("wrote markdown to"));
        let written = fs::read_to_string(&out_path).unwrap();
        assert!(written.starts_with("---\n"));
    }

    #[test]
    fn emit_markdown_rejects_invalid_json() {
        let file = write_temp_file("not json");
        let error = emit_markdown(
            file.path(),
            None,
            mif_frontmatter::FrontmatterShape::V1Canonical,
        )
        .unwrap_err();
        assert!(matches!(error, CliError::Json { .. }));
    }

    #[test]
    fn emit_markdown_rejects_json_ld_input_whose_fields_do_not_survive_the_trip() {
        // Regression: emit_markdown used to only check the DERIVED
        // markdown's own stability (mif_frontmatter::roundtrip_lossless on
        // its own output), never fidelity to the ORIGINAL JSON-LD — the
        // exact bug class fixed for roundtrip/emit-jsonld, but missed here
        // on the first pass since emit_markdown has a separate code path.
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:ee4a4a1e-9424-5575-b1be-8b1f80e13172",
                "conceptType": "semantic",
                "content": "Test content.",
                "timestamp": "2026-01-01T00:00:00Z"
            }"#,
        );
        let error = emit_markdown(
            file.path(),
            None,
            mif_frontmatter::FrontmatterShape::V1Canonical,
        )
        .unwrap_err();
        assert!(matches!(error, CliError::Frontmatter(_)));
    }

    #[test]
    fn json_serialize_error_maps_to_a_versioned_problem_type() {
        let source = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let error = CliError::JsonSerialize { source };
        let problem = error.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-cli-json-serialize-failure/v1"
        );
        assert_eq!(problem.status, 500);
        assert_eq!(problem.exit_code, Some(1));
    }

    #[test]
    fn frontmatter_shape_try_from_rejects_an_unknown_shape_name() {
        let error = mif_frontmatter::FrontmatterShape::try_from("PreProjected").unwrap_err();
        assert!(matches!(
            error,
            mif_frontmatter::FrontmatterError::UnknownShape(_)
        ));
    }

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
        assert!(message.contains("model=sentence-transformers/all-MiniLM-L6-v2"));
        assert!(message.contains("normalized=true"));
        assert!(message.contains("stored=true"));

        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        let stored = store
            .get("urn:mif:89886348-4773-5732-88b5-84366da76468")
            .unwrap()
            .unwrap();
        assert_eq!(stored.dim, 384);
    }

    #[test]
    fn ingest_accepts_a_conformant_jsonld_document() {
        warm_embedding_model_cache();
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:992e1654-6276-56a1-b993-3e529112a8ac",
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
        assert!(
            store
                .get("urn:mif:992e1654-6276-56a1-b993-3e529112a8ac")
                .unwrap()
                .is_some()
        );
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
id: 9c2bd382-34e9-52cf-b952-7fe33b0069ad
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
id: bf762c6d-debd-5113-9d65-73ddb163b89f
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
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-document/v1"
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

        let not_found =
            CliError::DocumentNotFound("urn:mif:eb333609-17f6-53ad-88bb-2faf7e0bcef8".to_string());
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
        let resolve_result = run_text(&Command::Ontology {
            command: OntologyCommand::Resolve {
                id: "base".to_string(),
                ontologies_dir: ontologies_dir.path().to_path_buf(),
            },
        });
        assert!(resolve_result.is_ok());

        let missing_file = tempfile::tempdir().unwrap().path().join("missing.json");
        let validate_result = run_text(&Command::Validate {
            file: missing_file,
            level: 1,
        });
        assert!(validate_result.is_err());

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        warm_embedding_model_cache();
        let doc_file = write_temp_file(VALID_MARKDOWN_FIXTURE);
        let ingest_result = run_text(&Command::Ingest {
            files: vec![doc_file.path().to_path_buf()],
            db_path: Some(db_path.clone()),
            changed_only: false,
            ignore: Vec::new(),
        });
        assert!(ingest_result.is_ok());

        let search_result = run_text(&Command::Search {
            query: "test content".to_string(),
            db_path: Some(db_path.clone()),
            extra_db_path: Vec::new(),
            limit: 5,
            id_prefix: None,
            source_prefix: None,
            namespace_prefix: None,
            entity_type: Vec::new(),
            exclude_entity_type: Vec::new(),
            concept_type: Vec::new(),
            exclude_concept_type: Vec::new(),
            mode: "hybrid".to_string(),
            show_passage: false,
        });
        assert!(search_result.is_ok());

        let find_similar_result = run_text(&Command::FindSimilar {
            id: "urn:mif:89886348-4773-5732-88b5-84366da76468".to_string(),
            db_path: Some(db_path.clone()),
            extra_db_path: Vec::new(),
            limit: 5,
        });
        assert!(find_similar_result.is_ok());

        let corpus_stats_result = run_text(&Command::CorpusStats {
            db_path: Some(db_path),
            extra_db_path: Vec::new(),
        });
        assert!(corpus_stats_result.is_ok());
    }

    fn ingest_fixture(db_path: &std::path::Path, id: &str, content: &str) {
        warm_embedding_model_cache();
        let file = write_temp_file(&format!(
            "---\nid: {}\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\n---\n\n{content}\n",
            mif_core::concept_uuid(id)
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

        let result = search("A furry pet cat", Some(&db_path), &[], &top(10)).unwrap();
        let first_line = result.lines().next().unwrap();
        assert!(first_line.ends_with("urn:mif:25b66725-81a9-5064-8d02-01eba8cb8cf6"));
    }

    #[test]
    fn search_reports_no_matches_on_an_empty_store() {
        warm_embedding_model_cache();
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        mif_store::VectorStore::open(&db_path).unwrap();

        let result = search("anything", Some(&db_path), &[], &top(10)).unwrap();
        assert_eq!(result, "(no matches)");
    }

    #[test]
    fn find_similar_excludes_the_anchor_document_itself() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        ingest_fixture(&db_path, "memory:a", "Cats are small domesticated felines.");
        ingest_fixture(&db_path, "memory:b", "Dogs are loyal domesticated canines.");

        let result = find_similar(
            "urn:mif:85c307af-aa7c-5f4f-9568-0ec847cf2b06",
            Some(&db_path),
            &[],
            10,
        )
        .unwrap();
        assert!(!result.contains(&mif_core::concept_urn("memory:a")));
        assert!(result.contains(&mif_core::concept_urn("memory:b")));
    }

    #[test]
    fn find_similar_reports_document_not_found() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        mif_store::VectorStore::open(&db_path).unwrap();

        let error = find_similar(
            "urn:mif:eb333609-17f6-53ad-88bb-2faf7e0bcef8",
            Some(&db_path),
            &[],
            10,
        )
        .unwrap_err();
        let problem = error.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-cli-document-not-found/v1"
        );
        assert_eq!(problem.status, 404);
    }

    #[test]
    fn corpus_stats_reports_count_and_dim() {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        assert_eq!(
            corpus_stats(Some(&db_path), &[]).unwrap(),
            format!("count=0 db={}", db_path.display())
        );

        ingest_fixture(&db_path, "memory:one", "Some content.");
        let result = corpus_stats(Some(&db_path), &[]).unwrap();
        assert!(result.contains("count=1"));
        assert!(result.contains("dim=384"));
    }

    #[test]
    fn search_with_extra_db_path_merges_and_ranks_across_roots() {
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");
        ingest_fixture(
            &db_path_a,
            "memory:cats",
            "Cats are small domesticated felines.",
        );
        ingest_fixture(
            &db_path_b,
            "memory:finance",
            "Quarterly revenue exceeded analyst expectations.",
        );

        let result = search(
            "A furry pet cat",
            Some(&db_path_a),
            std::slice::from_ref(&db_path_b),
            &top(10),
        )
        .unwrap();
        let first_line = result.lines().next().unwrap();
        assert!(first_line.contains("urn:mif:25b66725-81a9-5064-8d02-01eba8cb8cf6"));
        assert!(first_line.contains(&format!("root={}", db_path_a.display())));
        assert!(result.contains("urn:mif:0cca8466-0a4a-5535-b5c0-f683edf1bfb1"));
    }

    #[test]
    fn find_similar_with_extra_db_path_excludes_the_anchor_from_every_root() {
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");
        ingest_fixture(
            &db_path_a,
            "memory:a",
            "Cats are small domesticated felines.",
        );
        ingest_fixture(
            &db_path_b,
            "memory:b",
            "Dogs are loyal domesticated canines.",
        );

        let result = find_similar(
            "urn:mif:85c307af-aa7c-5f4f-9568-0ec847cf2b06",
            Some(&db_path_a),
            std::slice::from_ref(&db_path_b),
            10,
        )
        .unwrap();
        assert!(!result.contains(&mif_core::concept_urn("memory:a")));
        assert!(result.contains(&mif_core::concept_urn("memory:b")));
    }

    #[test]
    fn find_similar_still_returns_a_real_match_when_the_anchor_id_collides_across_every_root() {
        // Regression test: over_fetch used to be sized as `limit + 1`
        // regardless of root count, on the assumption that the anchor id
        // occupies exactly one slot. Once the same id is ingested into every
        // queried root (a realistic multi-root scenario -- see
        // RootedMatch's doc comment), the anchor occupies one slot PER root
        // it lives in, so a tight `limit` could get crowded out entirely by
        // duplicate anchor copies before the exclusion filter ever ran,
        // silently returning zero results even though a real match exists.
        // No embedder needed here -- upsert crafted vectors directly.
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");

        // Anchor lives in BOTH roots (identical vector, so both copies tie
        // for the highest possible cosine similarity, 1.0, against
        // themselves).
        mif_store::VectorStore::open(&db_path_a)
            .unwrap()
            .upsert("anchor", &[1.0, 0.0], "h", "t")
            .unwrap();
        mif_store::VectorStore::open(&db_path_b)
            .unwrap()
            .upsert("anchor", &[1.0, 0.0], "h", "t")
            .unwrap();
        // A real, genuinely-similar (but not identical) match, ranked just
        // behind the two anchor copies.
        mif_store::VectorStore::open(&db_path_b)
            .unwrap()
            .upsert("match", &[0.9, 0.1], "h", "t")
            .unwrap();

        let result = find_similar(
            "anchor",
            Some(&db_path_a),
            std::slice::from_ref(&db_path_b),
            1,
        )
        .unwrap();

        assert!(
            result.contains("match"),
            "expected the real match to survive anchor-collision exclusion, got: {result}"
        );
    }

    #[test]
    fn find_similar_with_extra_db_path_finds_the_anchor_in_a_non_primary_root() {
        // The anchor lives only in --extra-db-path, not the primary
        // --db-path root; multi_root_get must still locate it there.
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        mif_store::VectorStore::open(&db_path_a).unwrap();
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");
        ingest_fixture(
            &db_path_b,
            "memory:a",
            "Cats are small domesticated felines.",
        );
        ingest_fixture(
            &db_path_b,
            "memory:b",
            "Dogs are loyal domesticated canines.",
        );

        let result = find_similar(
            "urn:mif:85c307af-aa7c-5f4f-9568-0ec847cf2b06",
            Some(&db_path_a),
            std::slice::from_ref(&db_path_b),
            10,
        )
        .unwrap();
        assert!(!result.contains(&mif_core::concept_urn("memory:a")));
        assert!(result.contains(&mif_core::concept_urn("memory:b")));
    }

    #[test]
    fn corpus_stats_with_extra_db_path_reports_a_total_and_a_per_root_breakdown() {
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        let db_dir_b = tempfile::tempdir().unwrap();
        let db_path_b = db_dir_b.path().join("vectors.db");
        ingest_fixture(&db_path_a, "memory:one", "Some content.");
        ingest_fixture(&db_path_b, "memory:two", "Other content.");
        ingest_fixture(&db_path_b, "memory:three", "More content.");

        let result = corpus_stats(Some(&db_path_a), std::slice::from_ref(&db_path_b)).unwrap();
        assert!(result.contains("total_count=3"));
        assert!(result.contains(&format!("count=1 dim=384 db={}", db_path_a.display())));
        assert!(result.contains(&format!("count=2 dim=384 db={}", db_path_b.display())));
    }

    #[test]
    fn corpus_stats_with_extra_db_path_treats_a_not_yet_created_root_as_zero() {
        let db_dir_a = tempfile::tempdir().unwrap();
        let db_path_a = db_dir_a.path().join("vectors.db");
        ingest_fixture(&db_path_a, "memory:one", "Some content.");
        let not_yet_created_dir = tempfile::tempdir().unwrap();
        let not_yet_created_path = not_yet_created_dir.path().join("vectors.db");

        let result = corpus_stats(
            Some(&db_path_a),
            std::slice::from_ref(&not_yet_created_path),
        )
        .unwrap();
        assert!(result.contains("total_count=1"));
        assert!(result.contains(&format!("count=0 db={}", not_yet_created_path.display())));
    }

    #[test]
    fn ingest_stores_a_summary_and_one_passage_per_section() {
        warm_embedding_model_cache();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let file = dir.path().join("cats.md");
        fs::write(
            &file,
            sectioned_document(
                "cats",
                "Cats",
                "Cats are small domesticated felines.",
                "The veterinary team looks after them.",
            ),
        )
        .unwrap();

        let message = ingest(&file, Some(&db_path)).unwrap();

        // Summary, the Scope section, and the Owner section; the
        // Relationships section is not embedded.
        assert!(message.contains("embedding_dim=384 passages=3 model="));
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.passage_stats().unwrap().passage_count, 3);
        let stats = corpus_stats(Some(&db_path), &[]).unwrap();
        assert!(
            stats.ends_with("passages=3 documents_without_passages=0 documents_without_text=0")
        );
    }

    #[test]
    fn search_shows_the_matching_passage_only_when_asked() {
        warm_embedding_model_cache();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let file = dir.path().join("cats.md");
        fs::write(
            &file,
            sectioned_document(
                "cats",
                "Cats",
                "Cats are small domesticated felines.",
                "Quarterly revenue exceeded analyst expectations.",
            ),
        )
        .unwrap();
        ingest(&file, Some(&db_path)).unwrap();
        let id = format!("urn:mif:{}", mif_core::concept_uuid("cats"));
        let query = "revenue beat what analysts expected this quarter";

        let plain = search(query, Some(&db_path), &[], &top(10)).unwrap();
        let shown = search(
            query,
            Some(&db_path),
            &[],
            &SearchOptions {
                show_passage: true,
                ..top(10)
            },
        )
        .unwrap();

        assert!(plain.ends_with(&id), "unexpected plain output: {plain}");
        assert!(
            shown.ends_with(&format!("{id}  [Owner]")),
            "unexpected output: {shown}"
        );
    }

    #[test]
    fn search_filters_by_source_and_namespace_prefix() {
        warm_embedding_model_cache();
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".mif")).unwrap();
        let db_path = dir.path().join(".mif").join("vectors.db");
        for (folder, name) in [("capabilities", "cats"), ("notes", "dogs")] {
            fs::create_dir(dir.path().join(folder)).unwrap();
            let file = dir.path().join(folder).join(format!("{name}.md"));
            fs::write(
                &file,
                sectioned_document(name, name, "Small domesticated animals.", "A team."),
            )
            .unwrap();
            ingest(&file, Some(&db_path)).unwrap();
        }
        let cats = format!("urn:mif:{}", mif_core::concept_uuid("cats"));
        let source_prefix = "capabilities/".to_string();
        let filtered = |filter: mif_store::SearchFilter| {
            let options = SearchOptions { filter, ..top(10) };
            search("domesticated animals", Some(&db_path), &[], &options).unwrap()
        };

        assert_eq!(
            filtered(mif_store::SearchFilter::default()).lines().count(),
            2
        );
        let by_source = filtered(mif_store::SearchFilter {
            source_prefix: Some(source_prefix),
            ..mif_store::SearchFilter::default()
        });
        assert_eq!(by_source.lines().count(), 1);
        assert!(by_source.ends_with(&cats));
        let by_namespace = filtered(mif_store::SearchFilter {
            namespace_prefix: Some("records/cat".to_string()),
            ..mif_store::SearchFilter::default()
        });
        assert_eq!(by_namespace.lines().count(), 1);
        assert!(by_namespace.ends_with(&cats));
    }

    #[test]
    fn ingest_of_a_directory_attempts_every_document_and_reports_failures() {
        warm_embedding_model_cache();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("out").join("vectors.db");
        let docs = dir.path().join("docs");
        fs::create_dir_all(docs.join("nested")).unwrap();
        fs::create_dir_all(docs.join(".hidden")).unwrap();
        fs::write(
            docs.join("a.md"),
            sectioned_document("a", "A", "Alpha.", "Team."),
        )
        .unwrap();
        fs::write(
            docs.join("nested").join("b.md"),
            sectioned_document("b", "B", "Beta.", "Team."),
        )
        .unwrap();
        fs::write(
            docs.join("broken.md"),
            "---\nid: not-a-uuid\n---\n\nBody.\n",
        )
        .unwrap();
        fs::write(docs.join("README.md"), "# No frontmatter\n").unwrap();
        fs::write(docs.join("data.json"), "{}").unwrap();
        fs::write(
            docs.join(".hidden").join("c.md"),
            sectioned_document("c", "C", "Gamma.", "Team."),
        )
        .unwrap();

        let error = ingest_paths(&[docs], Some(&db_path), false, &[]).unwrap_err();

        assert!(matches!(
            error,
            CliError::IngestBatch {
                failed: 1,
                total: 3
            }
        ));
        assert_eq!(error.to_problem().exit_code, Some(2));
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 2);
    }

    #[test]
    fn ingest_with_changed_only_skips_what_the_store_already_holds() {
        warm_embedding_model_cache();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let file = dir.path().join("a.md");
        fs::write(&file, sectioned_document("a", "A", "Alpha.", "Team.")).unwrap();
        let files = [file.clone()];

        let first = ingest_paths(&files, Some(&db_path), true, &[]).unwrap();
        let second = ingest_paths(&files, Some(&db_path), true, &[]).unwrap();
        fs::write(
            &file,
            sectioned_document("a", "A", "Alpha, revised.", "Team."),
        )
        .unwrap();
        let third = ingest_paths(&files, Some(&db_path), true, &[]).unwrap();
        let unconditional = ingest_paths(&files, Some(&db_path), false, &[]).unwrap();

        assert!(first.contains("stored=true"));
        assert!(second.contains(": unchanged stored=false (id=urn:mif:"));
        assert!(third.contains("stored=true"));
        assert!(unconditional.contains("stored=true"));
    }

    #[test]
    fn ingest_of_several_files_reports_a_total() {
        warm_embedding_model_cache();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let files: Vec<PathBuf> = ["a", "b"]
            .iter()
            .map(|name| {
                let file = dir.path().join(format!("{name}.md"));
                fs::write(&file, sectioned_document(name, name, "Text.", "Team.")).unwrap();
                file
            })
            .collect();

        let message = ingest_paths(&files, Some(&db_path), false, &[]).unwrap();

        assert_eq!(
            message,
            format!("ingested 2 documents (db={})", db_path.display())
        );
    }

    /// A store holding three documents by id, with no model involved.
    fn store_with(db_path: &Path, ids: &[&str]) {
        let store = mif_store::VectorStore::open(db_path).unwrap();
        for id in ids {
            store.upsert(id, &[1.0, 0.0], "h", "t").unwrap();
        }
    }

    #[test]
    fn remove_deletes_one_document_and_says_when_there_was_none() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        store_with(&db_path, &["urn:mif:a", "urn:mif:b"]);

        assert_eq!(
            remove("urn:mif:a", Some(&db_path)).unwrap(),
            format!("urn:mif:a: removed (db={})", db_path.display())
        );
        assert_eq!(
            remove("urn:mif:a", Some(&db_path)).unwrap(),
            format!("urn:mif:a: not stored (db={})", db_path.display())
        );
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.ids().unwrap(), vec!["urn:mif:b"]);
    }

    #[test]
    fn prune_removes_documents_that_no_file_under_the_root_carries() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let root = dir.path().join("vault");
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("short.md"), "---\nid: kept-short\n---\n\nBody.\n").unwrap();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(
            root.join("nested").join("literal.md"),
            "---\n'@id': urn:mif:kept-literal\n---\n\nBody.\n",
        )
        .unwrap();
        // `ingest` never reads a hidden directory, so a copy left in one
        // (a trash folder, say) does not keep its document.
        fs::write(
            root.join(".hidden").join("copy.md"),
            "---\nid: gone-a\n---\n\nBody.\n",
        )
        .unwrap();
        fs::write(root.join("doc.json"), r#"{"@id": "urn:mif:kept-json"}"#).unwrap();
        fs::write(root.join("plain.md"), "# No frontmatter\n").unwrap();
        fs::write(root.join("other.json"), "not json").unwrap();
        fs::write(root.join("binary.json"), [0xff_u8, 0xfe, 0x00]).unwrap();
        store_with(
            &db_path,
            &[
                "urn:mif:kept-short",
                "urn:mif:kept-literal",
                "urn:mif:kept-json",
                "urn:mif:gone-b",
                "urn:mif:gone-a",
            ],
        );

        let dry = prune(&root, Some(&db_path), true, &[]).unwrap();
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 5);
        assert!(dry.starts_with("would prune urn:mif:gone-a\nwould prune urn:mif:gone-b\n"));
        assert!(dry.contains("would prune 2 documents, 3 found under"));

        let done = prune(&root, Some(&db_path), false, &[]).unwrap();
        assert!(done.starts_with("pruned urn:mif:gone-a\npruned urn:mif:gone-b\n"));
        assert_eq!(
            store.ids().unwrap(),
            vec![
                "urn:mif:kept-json",
                "urn:mif:kept-literal",
                "urn:mif:kept-short"
            ]
        );
    }

    #[test]
    fn prune_refuses_a_root_with_no_documents() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let root = dir.path().join("empty");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("plain.md"), "# No frontmatter\n").unwrap();
        store_with(&db_path, &["urn:mif:a"]);

        let error = prune(&root, Some(&db_path), false, &[]).unwrap_err();

        assert!(matches!(error, CliError::PruneEmptyRoot { .. }));
        let problem = error.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-cli-prune-empty-root/v1"
        );
        assert_eq!(problem.status, 400);
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn prune_stops_on_frontmatter_it_cannot_read_and_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let root = dir.path().join("vault");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("ok.md"), "---\nid: kept\n---\n\nBody.\n").unwrap();
        fs::write(root.join("bad.md"), "---\nid: [unclosed\n---\n\nBody.\n").unwrap();
        store_with(&db_path, &["urn:mif:kept", "urn:mif:maybe-bad"]);

        let error = prune(&root, Some(&db_path), false, &[]).unwrap_err();

        assert!(matches!(error, CliError::Frontmatter(_)));
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(store.count().unwrap(), 2);
    }

    #[test]
    fn ingest_batch_error_maps_to_its_own_problem_type() {
        let problem = CliError::IngestBatch {
            failed: 2,
            total: 5,
        }
        .to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/mif-cli-ingest-batch-failed/v1"
        );
        assert_eq!(problem.status, 422);
        assert_eq!(problem.detail, "2 of 5 documents were not ingested");
    }

    #[test]
    fn passage_suffix_is_empty_unless_asked_for_and_present() {
        assert_eq!(passage_suffix(Some("Scope"), true), "  [Scope]");
        assert_eq!(passage_suffix(Some("Scope"), false), "");
        assert_eq!(passage_suffix(None, true), "");
    }

    #[test]
    fn ingest_with_changed_only_stores_a_moved_document_under_its_new_source() {
        warm_embedding_model_cache();
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".mif")).unwrap();
        fs::create_dir_all(dir.path().join("active")).unwrap();
        fs::create_dir_all(dir.path().join("retired")).unwrap();
        let db_path = dir.path().join(".mif").join("vectors.db");
        let before = dir.path().join("active").join("a.md");
        let after = dir.path().join("retired").join("a.md");
        fs::write(&before, sectioned_document("a", "A", "Alpha.", "Team.")).unwrap();
        ingest_paths(std::slice::from_ref(&before), Some(&db_path), true, &[]).unwrap();

        fs::rename(&before, &after).unwrap();
        let moved = ingest_paths(std::slice::from_ref(&after), Some(&db_path), true, &[]).unwrap();

        assert!(moved.contains("stored=true"), "unexpected output: {moved}");
        let under = |prefix: &str| {
            let options = SearchOptions {
                filter: mif_store::SearchFilter {
                    source_prefix: Some(prefix.to_string()),
                    ..mif_store::SearchFilter::default()
                },
                ..top(10)
            };
            search("alpha", Some(&db_path), &[], &options).unwrap()
        };
        assert_eq!(under("active/"), "(no matches)");
        assert_eq!(under("retired/").lines().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn prune_keeps_a_document_that_is_a_symbolic_link_to_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("vectors.db");
        let root = dir.path().join("vault");
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(root.join("plain.md"), "---\nid: plain\n---\n\nBody.\n").unwrap();
        fs::write(elsewhere.join("real.md"), "---\nid: linked\n---\n\nBody.\n").unwrap();
        std::os::unix::fs::symlink(elsewhere.join("real.md"), root.join("link.md")).unwrap();
        // A link back to the root must not send the walk round in circles.
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        store_with(
            &db_path,
            &["urn:mif:plain", "urn:mif:linked", "urn:mif:gone"],
        );

        let done = prune(&root, Some(&db_path), false, &[]).unwrap();

        assert!(done.starts_with("pruned urn:mif:gone\n"), "{done}");
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        assert_eq!(
            store.ids().unwrap(),
            vec!["urn:mif:linked", "urn:mif:plain"]
        );
    }
}

#[cfg(test)]
mod ignore_tests {
    use std::fs;

    use super::{CliError, Ignore, prune, walk};

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "keep.md",
            "keep.draft.md",
            "notes/a.md",
            "notes/b.draft.md",
            "notes/archive/old.md",
            "archive/top.md",
            "deep/archive/low.md",
            ".trash/gone.md",
        ] {
            let path = dir.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let id = file.replace(['/', '.'], "-");
            fs::write(path, format!("---\nid: {id}\n---\n\nBody.\n")).unwrap();
        }
        dir
    }

    fn walked(dir: &tempfile::TempDir, patterns: &[&str]) -> Vec<String> {
        let patterns: Vec<String> = patterns.iter().map(ToString::to_string).collect();
        let mut found = Vec::new();
        walk(
            dir.path(),
            dir.path(),
            &Ignore::new(&patterns).unwrap(),
            &mut found,
        )
        .unwrap();
        found
            .iter()
            .map(|path| {
                path.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    #[test]
    fn a_walk_skips_hidden_directories_and_nothing_else_by_default() {
        let dir = tree();

        assert_eq!(
            walked(&dir, &[]),
            vec![
                "archive/top.md",
                "deep/archive/low.md",
                "keep.draft.md",
                "keep.md",
                "notes/a.md",
                "notes/archive/old.md",
                "notes/b.draft.md",
            ]
        );
    }

    #[test]
    fn a_pattern_without_a_slash_matches_a_name_at_any_depth() {
        let dir = tree();

        assert_eq!(
            walked(&dir, &["*.draft.md"]),
            vec![
                "archive/top.md",
                "deep/archive/low.md",
                "keep.md",
                "notes/a.md",
                "notes/archive/old.md",
            ]
        );
        assert_eq!(
            walked(&dir, &["archive/"]),
            vec!["keep.draft.md", "keep.md", "notes/a.md", "notes/b.draft.md"]
        );
    }

    #[test]
    fn a_pattern_with_a_slash_matches_the_path_from_the_walked_directory() {
        let dir = tree();

        assert_eq!(
            walked(&dir, &["notes/archive/**", "/archive"]),
            vec![
                "deep/archive/low.md",
                "keep.draft.md",
                "keep.md",
                "notes/a.md",
                "notes/b.draft.md",
            ]
        );
        // `*` stays within one path component; `**` crosses them.
        assert_eq!(
            walked(&dir, &["*/*.md"]),
            vec![
                "deep/archive/low.md",
                "keep.draft.md",
                "keep.md",
                "notes/archive/old.md",
            ]
        );
        // `**/` also stands for no directory at all.
        assert!(walked(&dir, &["**/*.md"]).is_empty());
    }

    #[test]
    fn an_unreadable_pattern_is_reported_with_the_pattern() {
        let error = Ignore::new(&["notes/[".to_string()]).err().unwrap();

        assert!(matches!(&error, CliError::InvalidIgnore { pattern, .. } if pattern == "notes/["));
        assert!(error.to_string().contains("notes/["));
    }

    #[test]
    fn prune_does_not_count_what_it_is_told_to_ignore() {
        let dir = tree();
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("vectors.db");
        let store = mif_store::VectorStore::open(&db_path).unwrap();
        for id in [
            "keep-md",
            "notes-a-md",
            "notes-archive-old-md",
            "-trash-gone-md",
        ] {
            store
                .upsert(&format!("urn:mif:{id}"), &[1.0, 0.0], "h", "t")
                .unwrap();
        }

        let dry = prune(
            dir.path(),
            Some(&db_path),
            true,
            &["notes/archive/**".to_string()],
        )
        .unwrap();

        assert!(dry.starts_with(
            "would prune urn:mif:-trash-gone-md\nwould prune urn:mif:notes-archive-old-md\n"
        ));
        assert_eq!(store.count().unwrap(), 4);
    }
}
