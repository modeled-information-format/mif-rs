//! Errors from loading, resolving, reviewing, indexing, or locking a
//! research-harness-template (rht) corpus.

use mif_problem::{
    Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem,
};

/// Errors from `mif-rh`'s engine core.
///
/// Variants split into two classes: those that produce a
/// [`crate::resolve::MapRecord`] anyway (a finding that classifies as
/// invalid/ambiguous/unresolved is still recorded, per rht's own
/// `resolve-ontology.sh` — see [`crate::resolve::resolve_finding`]) and
/// those below, which mean no record could be produced at all (the file
/// itself is unreadable, the catalog is missing, or an ontology definition
/// is broken). [`crate::review::review`] folds the latter into its
/// reconciliation "gap" count rather than aborting the whole review.
#[derive(Debug, thiserror::Error)]
pub enum MifRhError {
    /// Failed to read a finding file.
    #[error("failed to read finding {path}: {source}")]
    FindingIo {
        /// The path that failed to read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A finding file was not valid JSON.
    #[error("finding {path} is not valid JSON: {source}")]
    FindingJson {
        /// The path that failed to parse.
        path: String,
        /// The underlying parse error.
        #[source]
        source: serde_json::Error,
    },
    /// A generic I/O failure reading a supporting file (ontology directory,
    /// map file, `harness.config.json`).
    #[error("failed to read {path}: {source}")]
    Io {
        /// The path that failed to read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A supporting file was not valid JSON.
    #[error("failed to parse {path} as JSON: {source}")]
    Json {
        /// The path that failed to parse.
        path: String,
        /// The underlying parse error.
        #[source]
        source: serde_json::Error,
    },
    /// An ontology pack YAML file failed to parse (the direct equivalent of
    /// `yq` failing to read an ontology's `extends`/`entity_types`/full
    /// YAML — a fail-closed abort, matching rht's own bash exit code 4).
    #[error("failed to parse ontology pack {path} as YAML: {source}")]
    OntologyPackYaml {
        /// The path that failed to parse.
        path: String,
        /// The underlying parse error.
        #[source]
        source: serde_norway::Error,
    },
    /// The catalog file (`.claude/enabled-packs.json`) is missing.
    #[error("catalog file {path} does not exist")]
    CatalogMissing {
        /// The missing catalog path.
        path: String,
    },
    /// The config file (`harness.config.json`) is missing.
    #[error("config file {path} does not exist")]
    ConfigMissing {
        /// The missing config path.
        path: String,
    },
    /// A topic directly binds an ontology id that is not cataloged, or pins
    /// a version that does not match the cataloged one.
    #[error("topic '{topic}' binds ontology '{id}', which is not cataloged or version-mismatched")]
    DirectBindingInvalid {
        /// The topic with the invalid binding.
        topic: String,
        /// The offending ontology id.
        id: String,
    },
    /// Resolving the transitive `extends` ancestry for an allowed ontology
    /// failed (an ancestor is missing from the supplied ontology-pack
    /// directory, or the `extends` graph is cyclic).
    #[error(transparent)]
    Ontology(#[from] mif_ontology::OntologyError),
    /// Building a dynamic `jsonschema` validator for a resolved entity
    /// type's `schema` field failed. Indicates a malformed ontology pack,
    /// not a bug in the finding being validated.
    #[error("entity type '{entity_type}' has a malformed validation schema: {detail}")]
    EntityTypeSchemaInvalid {
        /// The offending entity type name.
        entity_type: String,
        /// The underlying schema compilation error, stringified (the
        /// original `jsonschema` error type is not `'static` and cannot be
        /// stored here directly).
        detail: String,
    },
    /// A `SQLite` index operation failed.
    #[error("sqlite index operation failed: {source}")]
    Index {
        /// The underlying `SQLite` error.
        #[source]
        source: rusqlite::Error,
    },
    /// Failed to open or write the exclusive review lock file.
    #[error("failed to acquire the review lock at {path}: {source}")]
    LockIo {
        /// The lock file path.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Another `review` run already holds the lock.
    #[error("another review is already in progress (lock held by pid {holder_pid})")]
    LockHeld {
        /// The PID recorded in the held lock file.
        holder_pid: u32,
    },
    /// Computing an embedding failed.
    #[error(transparent)]
    Embed(#[from] mif_embed::EmbedError),
}

impl MifRhError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::FindingIo { .. } => ProblemMeta {
                slug: "finding-io",
                version: "v1",
                title: "Failed to read a finding file",
                status: 500,
                exit_code: 2,
            },
            Self::FindingJson { .. } => ProblemMeta {
                slug: "finding-invalid-json",
                version: "v1",
                title: "Finding file is not valid JSON",
                status: 400,
                exit_code: 2,
            },
            Self::Io { .. } => ProblemMeta {
                slug: "io",
                version: "v1",
                title: "Failed to read a supporting file",
                status: 500,
                exit_code: 1,
            },
            Self::Json { .. } => ProblemMeta {
                slug: "invalid-json",
                version: "v1",
                title: "Supporting file is not valid JSON",
                status: 400,
                exit_code: 1,
            },
            Self::OntologyPackYaml { .. } => ProblemMeta {
                slug: "ontology-pack-invalid-yaml",
                version: "v1",
                title: "Ontology pack YAML failed to parse",
                status: 422,
                exit_code: 4,
            },
            Self::CatalogMissing { .. } => ProblemMeta {
                slug: "catalog-missing",
                version: "v1",
                title: "Ontology catalog file does not exist",
                status: 404,
                exit_code: 3,
            },
            Self::ConfigMissing { .. } => ProblemMeta {
                slug: "config-missing",
                version: "v1",
                title: "Harness config file does not exist",
                status: 404,
                exit_code: 2,
            },
            Self::DirectBindingInvalid { .. } => ProblemMeta {
                slug: "direct-binding-invalid",
                version: "v1",
                title: "Topic binds an uncataloged or version-mismatched ontology",
                status: 422,
                exit_code: 1,
            },
            Self::Ontology(_) => ProblemMeta {
                slug: "delegated-ontology",
                version: "v1",
                title: "Delegated ontology error",
                status: 500,
                exit_code: 4,
            },
            Self::EntityTypeSchemaInvalid { .. } => ProblemMeta {
                slug: "entity-type-schema-invalid",
                version: "v1",
                title: "Entity type validation schema is malformed",
                status: 422,
                exit_code: 4,
            },
            Self::Index { .. } => ProblemMeta {
                slug: "index-failure",
                version: "v1",
                title: "A SQLite index operation failed",
                status: 500,
                exit_code: 1,
            },
            Self::LockIo { .. } => ProblemMeta {
                slug: "lock-io",
                version: "v1",
                title: "Failed to acquire the review lock file",
                status: 500,
                exit_code: 2,
            },
            Self::LockHeld { .. } => ProblemMeta {
                slug: "lock-held",
                version: "v1",
                title: "Another review is already in progress",
                status: 409,
                exit_code: 2,
            },
            Self::Embed(_) => ProblemMeta {
                slug: "delegated-embed",
                version: "v1",
                title: "Delegated embedding error",
                status: 500,
                exit_code: 1,
            },
        }
    }
}

/// The `(suggested_fix, code_action)` pair for every variant that carries
/// its own static remediation text (everything except the delegated
/// `Ontology`/`Embed` variants and the IO-classified variants, which
/// `to_problem` handles separately).
fn fix_and_action(error: &MifRhError) -> (SuggestedFix, CodeAction) {
    match error {
        MifRhError::OntologyPackYaml { .. } => (
            SuggestedFix::new(
                "Fix the YAML syntax error in the ontology pack, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the malformed ontology pack YAML",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::CatalogMissing { .. } => (
            SuggestedFix::new(
                "Run rht's scripts/sync-packs.sh (or mif-rh-cli's equivalent) to generate the \
                 catalog, then retry.",
                Applicability::MachineApplicable,
            ),
            CodeAction::new(
                "Generate the missing catalog",
                "quickfix",
                Applicability::MachineApplicable,
            ),
        ),
        MifRhError::ConfigMissing { .. } => (
            SuggestedFix::new(
                "Supply the correct --config path to harness.config.json, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Correct the --config path",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::DirectBindingInvalid { .. } => (
            SuggestedFix::new(
                "Catalog the missing ontology, correct its pinned version, or remove the \
                 invalid topic binding, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the topic's ontology binding",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::EntityTypeSchemaInvalid { .. } => (
            SuggestedFix::new(
                "Fix the entity type's schema field in the ontology pack, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the entity type's schema",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::Index { .. } => (
            SuggestedFix::new(
                "This indicates a corrupt or inaccessible index database. Delete it and \
                 rebuild with `mif-rh-cli review --build-index`, then retry.",
                Applicability::Unspecified,
            ),
            CodeAction::new("Rebuild the index", "quickfix", Applicability::Unspecified),
        ),
        MifRhError::LockHeld { .. } => (
            SuggestedFix::new(
                "Wait for the in-progress review to finish, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new("Wait and retry", "quickfix", Applicability::MaybeIncorrect),
        ),
        // FindingJson/Json carry no additional remediation beyond the
        // error message itself; the caller (`to_problem`) never invokes
        // this helper for the delegated or IO-classified variants.
        MifRhError::FindingJson { .. } | MifRhError::Json { .. } => (
            SuggestedFix::new(
                "Correct the file so it is valid JSON, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the JSON syntax error",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::FindingIo { .. }
        | MifRhError::Io { .. }
        | MifRhError::LockIo { .. }
        | MifRhError::Ontology(_)
        | MifRhError::Embed(_) => unreachable!(
            "to_problem handles the IO-classified and delegated variants before calling \
             fix_and_action"
        ),
    }
}

impl ToProblem for MifRhError {
    fn to_problem(&self) -> ProblemDetails {
        match self {
            Self::Ontology(inner) => inner.to_problem(),
            Self::Embed(inner) => inner.to_problem(),
            Self::FindingIo { source, .. }
            | Self::Io { source, .. }
            | Self::LockIo { source, .. } => {
                let (status, fix, action) = mif_problem::classify_io_error(source);
                let mut problem = self
                    .meta()
                    .into_details(env!("CARGO_PKG_NAME"), self.to_string());
                problem.status = status;
                problem.with_suggested_fix(fix).with_code_action(action)
            },
            _ => {
                let (fix, action) = fix_and_action(self);
                self.meta()
                    .into_details(env!("CARGO_PKG_NAME"), self.to_string())
                    .with_suggested_fix(fix)
                    .with_code_action(action)
            },
        }
    }
}

impl From<rusqlite::Error> for MifRhError {
    fn from(source: rusqlite::Error) -> Self {
        Self::Index { source }
    }
}
