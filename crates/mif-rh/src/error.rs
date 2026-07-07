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
    /// A value could not be serialized to JSON for an atomic write. This
    /// indicates a bug in the value's `Serialize` implementation (e.g. a
    /// non-finite float) rather than anything a caller can fix by changing
    /// its input.
    #[error("failed to serialize {path} as JSON: {source}")]
    JsonSerialize {
        /// The path the value was being written to.
        path: String,
        /// The underlying serialization error.
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
    /// Splitting a report's frontmatter/body failed (missing frontmatter,
    /// malformed YAML, or a non-mapping frontmatter document).
    #[error(transparent)]
    Frontmatter(#[from] mif_frontmatter::FrontmatterError),
    /// A report schema (or one of its `$ref` dependencies) was not valid
    /// JSON, or the `jsonschema` validator could not be compiled from it.
    #[error("failed to compile schema {path}: {detail}")]
    SchemaCompilation {
        /// The schema file that failed to compile.
        path: String,
        /// The underlying compilation error, stringified (the original
        /// `jsonschema` error type is not `'static`).
        detail: String,
    },
    /// A `$ref` dependency schema has no `$id`, so it cannot be registered
    /// for the main schema to resolve references against.
    #[error("schema {path} has no $id — it cannot be registered as a $ref target")]
    RefSchemaMissingId {
        /// The dependency schema file with no `$id`.
        path: String,
    },
    /// A projected report failed validation against its schema.
    #[error("{path} failed schema validation against {schema_path}: {detail}")]
    SchemaValidationFailed {
        /// The report file that failed validation.
        path: String,
        /// The schema it was validated against.
        schema_path: String,
        /// Every validation error, joined for display.
        detail: String,
    },
    /// A manifest toggle's value is not one of the allowed values.
    #[error("{field} must be one of {allowed} (got '{value}')")]
    InvalidToggleValue {
        /// The toggled field's name.
        field: String,
        /// The rejected value.
        value: String,
        /// The pipe-joined list of allowed values.
        allowed: String,
    },
    /// No source content was available from `--content-file`,
    /// `--content`, or stdin.
    #[error("empty content (provide --content-file, --content, or stdin)")]
    EmptySourceContent,
    /// A `pack-toggle` target is not declared in `harness.config.json`'s
    /// `packs[]` array. Distinct from [`Self::PackNotFound`] (a
    /// `bump-version --pack` target with no `packs/<family>/<name>/`
    /// directory on disk) — this is a config-declaration check, not a
    /// filesystem one.
    #[error("pack '{name}' is not declared in {path} packs[] — declare it first")]
    PackNotDeclared {
        /// The undeclared pack name.
        name: String,
        /// The manifest path checked.
        path: String,
    },
    /// A findings directory has no `*.json` files to build a graph/index
    /// from.
    #[error("no finding JSON in {path}")]
    NoFindingsFound {
        /// The empty findings directory.
        path: String,
    },
    /// Every finding in a directory is falsified — nothing to synthesize.
    #[error("no surviving findings to synthesize in {path}")]
    NoSurvivingFindings {
        /// The findings directory with no surviving findings.
        path: String,
    },
    /// A synthesized artifact has no sections, finding refs, or sources —
    /// there is nothing publishable to render.
    #[error(
        "artifact from {path} has no publishable content (no surviving findings, or no citations to cite)"
    )]
    ArtifactNotPublishable {
        /// The findings directory the artifact was synthesized from.
        path: String,
    },
    /// One or more findings in a corpus import lack a provenance block
    /// (SPEC §8a: provenance must survive an import).
    #[error(
        "{count} finding(s) lack a provenance block; import aborted (provenance must be preserved)"
    )]
    MissingProvenance {
        /// How many findings lack provenance.
        count: usize,
        /// The paths of the findings missing provenance.
        paths: Vec<String>,
    },
    /// A session reconciliation's known-good sample finding failed schema
    /// validation — the schema/toolchain itself is broken. Must never be
    /// read as "every finding is invalid" (that would re-run an entire
    /// expensive research session).
    #[error(
        "the known-good sample finding at {sample_path} failed schema validation — the schema/toolchain is broken, refusing to emit a plan"
    )]
    ReconcileEnvironmentBroken {
        /// The sample finding path that unexpectedly failed to validate.
        sample_path: String,
    },
    /// A topic README build/check was requested for a topic with no entry
    /// in `harness.config.json`'s `topics[]`.
    #[error("topic '{topic}' is not registered in {config_path}")]
    TopicNotRegistered {
        /// The unregistered topic id.
        topic: String,
        /// The manifest path checked.
        config_path: String,
    },
    /// A concordance file parsed but is not a valid graph (no `.nodes`
    /// array) — the corpus atlas is a projection of the spine, and there
    /// is nothing to project without one.
    #[error("concordance is not a valid graph (no .nodes array): {path}")]
    InvalidConcordance {
        /// The invalid concordance path.
        path: String,
    },
    /// The shippable-typing gate's `ontology-map.json` is missing or
    /// present-but-unparseable (not a JSON array of records) — the gate
    /// cannot prove any shippable finding is typed, so it fails closed
    /// rather than passing vacuously (every per-finding lookup would
    /// otherwise silently resolve to "no record").
    #[error(
        "ontology-map.json {reason} for topic '{topic}' — synthesis BLOCKED (fail closed): {path}"
    )]
    OntologyMapUnusable {
        /// The ontology-map path checked.
        path: String,
        /// The topic (derived from the reports-dir name) named in the
        /// operator unblock hint.
        topic: String,
        /// Either `"is missing"` or `"is unparseable or not a record array"`.
        reason: String,
    },
    /// A finding under `<topic>/findings/` failed to parse while building
    /// the corpus-wide active-`@id`/relationship-target universes for the
    /// relationship-targets gate. Hard-fails the whole gate (never silently
    /// dropped) — an unparseable file could otherwise hide a real dangling
    /// target elsewhere in the corpus.
    #[error("finding is not valid JSON, cannot check its relationship targets: {path}")]
    RelationshipTargetFindingUnparseable {
        /// The unparseable finding path.
        path: String,
    },
    /// A `subtype_of` chain across the loaded ontology registry revisits
    /// its own starting type — a cycle. User-authored ontology data must
    /// never drive unbounded recursion during the concordance's transitive
    /// supertype closure.
    #[error("subtype_of graph contains a cycle involving entity type '{entity_type}'")]
    SubtypeOfCycle {
        /// The entity type where the cycle was detected.
        entity_type: String,
    },
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
    /// A suggestion queue file on disk belongs to a different topic than
    /// the one being upserted — a copied/renamed queue file or a
    /// wrong-path caller, which must not silently mix topics' entries.
    #[error("suggestion queue at {path} belongs to topic '{found}', not '{expected}'")]
    QueueTopicMismatch {
        /// The queue file path.
        path: String,
        /// The topic the caller is upserting.
        expected: String,
        /// The topic recorded inside the queue file.
        found: String,
    },
    /// Computing an embedding failed.
    #[error(transparent)]
    Embed(#[from] mif_embed::EmbedError),
    /// Failed to fetch a file (the registry index, or a vendored ontology)
    /// from the resolved ontology source (a local directory or an http(s)
    /// base URL).
    #[error("failed to fetch from ontology source {registry_source}: {detail}")]
    RegistryFetch {
        /// The resolved source (directory path or URL base). Named
        /// `registry_source`, not `source`: `thiserror` treats a field
        /// literally named `source` as the error-chain cause and requires
        /// it to implement `std::error::Error`, which a plain `String`
        /// does not.
        registry_source: String,
        /// A human-readable failure detail.
        detail: String,
    },
    /// The registry index (`index.json`) was not valid JSON or did not
    /// match the expected shape.
    #[error("ontology registry index at {registry_source} is malformed: {detail}")]
    RegistryIndexInvalid {
        /// The resolved source the index was read from. See
        /// [`Self::RegistryFetch`]'s doc comment for why this is
        /// `registry_source`, not `source`.
        registry_source: String,
        /// A human-readable failure detail.
        detail: String,
    },
    /// A requested or `extends`-ancestor ontology id has no entry in the
    /// registry index — it has no canonical definition yet.
    #[error(
        "ontology '{id}' is not in the registry index — it has no canonical definition yet \
         (author one with the `ontology author` subcommand)"
    )]
    OntologyNotInRegistry {
        /// The unresolvable ontology id.
        id: String,
    },
    /// The registry index's sha256 no longer matches the value pinned in
    /// `ontologies.lock.json` for the same source (trust-on-first-use, then
    /// pin) — the trust root moved.
    #[error(
        "registry index sha256 changed from the pinned value for source {registry_source} \
         (pinned {pinned}, got {got}) — refusing to trust a moved index (clear index_sha256 \
         in the lock to re-pin deliberately)"
    )]
    IndexPinMismatch {
        /// The source whose index changed. See [`Self::RegistryFetch`]'s
        /// doc comment for why this is `registry_source`, not `source`.
        registry_source: String,
        /// The previously pinned index sha256.
        pinned: String,
        /// The newly fetched index's sha256.
        got: String,
    },
    /// A fetched ontology file's sha256 did not match the registry index's
    /// pinned value — refusing to vendor a file that does not match the
    /// trusted hash (fail-closed).
    #[error("checksum mismatch for ontology '{id}' ({file}): expected {expected}, got {got}")]
    ChecksumMismatch {
        /// The ontology id being vendored.
        id: String,
        /// The index's declared file name.
        file: String,
        /// The expected (pinned) sha256.
        expected: String,
        /// The sha256 actually computed from the fetched bytes.
        got: String,
    },
    /// The registry index named an unsafe (non-bare) file path for an
    /// ontology — a poisoned index could otherwise escape the vendored
    /// packs directory.
    #[error(
        "registry index entry for '{id}' has an unsafe file path: '{file}' (must be a bare \
         filename)"
    )]
    UnsafeIndexPath {
        /// The ontology id whose index entry is unsafe.
        id: String,
        /// The offending file path.
        file: String,
    },
    /// A registry-discovered ontology id is not a bare, lowercase slug — a
    /// poisoned or malformed index entry that could otherwise escape its
    /// intended vendored directory once written into `harness.config.json`.
    #[error("registry index declares a malformed ontology id: '{id}' (refusing, fail-closed)")]
    MalformedOntologyId {
        /// The malformed id.
        id: String,
    },
    /// `harness.config.json`'s `.ontologies` field exists but is not an
    /// array — refusing to guess how to append a discovered ontology to it.
    #[error("{path}'s .ontologies is not an array: {detail}")]
    ConfigMalformed {
        /// The config path.
        path: String,
        /// A human-readable failure detail.
        detail: String,
    },
    /// A topic's `ontology-map.json` carries no typed entity types to
    /// mine an ontology draft from.
    #[error(
        "no entity types found in reports/{topic}/ontology-map.json — nothing to draft an \
         ontology from"
    )]
    NoEntityTypesFound {
        /// The topic whose map carried no typed entities.
        topic: String,
    },
    /// An `expansion-candidates` output file carries no clusters to draft
    /// candidate types from.
    #[error("no clusters in {path} — nothing to draft an ontology from")]
    NoClustersFound {
        /// The clusters file with no clusters.
        path: String,
    },
    /// A version string is not well-formed `X.Y.Z` semver.
    #[error("version is not well-formed semver: {value}")]
    VersionNotSemver {
        /// The offending value.
        value: String,
    },
    /// A file expected to carry a `.version` field has none.
    #[error("{path} has no .version")]
    VersionMissing {
        /// The file with no version.
        path: String,
    },
    /// The requested new version equals the current one.
    #[error("new version equals current ({value}); nothing to bump")]
    VersionUnchanged {
        /// The unchanged value.
        value: String,
    },
    /// A `--pack` component name resolves to no directory under
    /// `packs/<family>/`.
    #[error(
        "pack '{name}' not found under packs/<family>/ (ontology packs version independently and are not bumpable here)"
    )]
    PackNotFound {
        /// The unresolved component name.
        name: String,
    },
    /// A `--pack` component name resolves to more than one directory.
    #[error("pack '{name}' is ambiguous: more than one packs/<family>/{name} directory exists")]
    PackAmbiguous {
        /// The ambiguous component name.
        name: String,
    },
    /// A pack is missing a file `bump_version` needs (its `plugin.json`,
    /// `SKILL.md`, or family doc section/row).
    #[error("pack '{name}': missing or malformed {path}")]
    PackFileMissing {
        /// The pack's component name.
        name: String,
        /// The missing or malformed file/section.
        path: String,
    },
    /// A pack's declared version is not well-formed semver.
    #[error("pack '{name}' {path} has no valid semver .version (got '{value}')")]
    PackVersionInvalid {
        /// The pack's component name.
        name: String,
        /// The pack's `plugin.json` path.
        path: String,
        /// The malformed value found.
        value: String,
    },
    /// A pack's current version is already ahead of the release being cut —
    /// bumping it would move it backward.
    #[error(
        "pack '{name}' is at {pack_version}, ahead of the new release {new_version} — refusing to move it backward"
    )]
    PackAheadOfRelease {
        /// The pack's component name.
        name: String,
        /// The pack's current version.
        pack_version: String,
        /// The release version being cut.
        new_version: String,
    },
    /// `CHANGELOG.md` has neither an `## [Unreleased]` anchor to insert a
    /// new section under, nor an existing section for the new version.
    #[error(
        "{path} has no '## [Unreleased]' anchor to insert the new version under (nor an existing section for it)"
    )]
    ChangelogAnchorMissing {
        /// The CHANGELOG path.
        path: String,
    },
    /// A post-write self-verification found a file that did not update to
    /// the expected new value.
    #[error("verification failed: {path} was not updated as expected")]
    VerificationFailed {
        /// The file that failed verification.
        path: String,
    },
}

impl MifRhError {
    // One arm per error variant, each a flat struct literal — length is
    // inherent to the variant count, not a complexity signal.
    #[allow(clippy::too_many_lines)]
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
                slug: "mif-rh-io",
                version: "v1",
                title: "Failed to read a supporting file",
                status: 500,
                exit_code: 1,
            },
            Self::Json { .. } => ProblemMeta {
                slug: "mif-rh-invalid-json",
                version: "v1",
                title: "Supporting file is not valid JSON",
                status: 400,
                exit_code: 1,
            },
            Self::JsonSerialize { .. } => ProblemMeta {
                slug: "json-serialize-failure",
                version: "v1",
                title: "A value could not be serialized to JSON",
                status: 500,
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
            Self::Frontmatter(_) => ProblemMeta {
                slug: "delegated-frontmatter",
                version: "v1",
                title: "Delegated frontmatter error",
                status: 500,
                exit_code: 4,
            },
            Self::SchemaCompilation { .. } => ProblemMeta {
                slug: "schema-compilation-failed",
                version: "v1",
                title: "Report schema failed to compile",
                status: 422,
                exit_code: 2,
            },
            Self::RefSchemaMissingId { .. } => ProblemMeta {
                slug: "ref-schema-missing-id",
                version: "v1",
                title: "Ref-target schema has no $id",
                status: 422,
                exit_code: 2,
            },
            Self::SchemaValidationFailed { .. } => ProblemMeta {
                slug: "schema-validation-failed",
                version: "v1",
                title: "Report failed schema validation",
                status: 422,
                exit_code: 1,
            },
            Self::InvalidToggleValue { .. } => ProblemMeta {
                slug: "invalid-toggle-value",
                version: "v1",
                title: "Manifest toggle value is not one of the allowed values",
                status: 422,
                exit_code: 2,
            },
            Self::EmptySourceContent => ProblemMeta {
                slug: "empty-source-content",
                version: "v1",
                title: "No source content was available from any input",
                status: 422,
                exit_code: 2,
            },
            Self::PackNotDeclared { .. } => ProblemMeta {
                slug: "pack-not-declared",
                version: "v1",
                title: "Pack is not declared in the harness manifest",
                status: 404,
                exit_code: 2,
            },
            Self::NoFindingsFound { .. } => ProblemMeta {
                slug: "no-findings-found",
                version: "v1",
                title: "Findings directory has no finding JSON",
                status: 404,
                exit_code: 2,
            },
            Self::NoSurvivingFindings { .. } => ProblemMeta {
                slug: "no-surviving-findings",
                version: "v1",
                title: "Every finding is falsified — nothing to synthesize",
                status: 422,
                exit_code: 1,
            },
            Self::ArtifactNotPublishable { .. } => ProblemMeta {
                slug: "artifact-not-publishable",
                version: "v1",
                title: "Synthesized artifact has no publishable content",
                status: 422,
                exit_code: 1,
            },
            Self::MissingProvenance { .. } => ProblemMeta {
                slug: "missing-provenance",
                version: "v1",
                title: "Corpus import contains findings with no provenance block",
                status: 422,
                exit_code: 1,
            },
            Self::ReconcileEnvironmentBroken { .. } => ProblemMeta {
                slug: "reconcile-environment-broken",
                version: "v1",
                title: "Known-good sample finding failed schema validation",
                status: 500,
                exit_code: 3,
            },
            Self::TopicNotRegistered { .. } => ProblemMeta {
                slug: "topic-not-registered",
                version: "v1",
                title: "Topic is not registered in harness.config.json",
                status: 422,
                exit_code: 2,
            },
            Self::InvalidConcordance { .. } => ProblemMeta {
                slug: "invalid-concordance",
                version: "v1",
                title: "Concordance is not a valid graph",
                status: 422,
                exit_code: 2,
            },
            Self::OntologyMapUnusable { .. } => ProblemMeta {
                slug: "ontology-map-unusable",
                version: "v1",
                title: "ontology-map.json is missing or unparseable — cannot prove typing",
                status: 422,
                exit_code: 3,
            },
            Self::RelationshipTargetFindingUnparseable { .. } => ProblemMeta {
                slug: "relationship-target-finding-unparseable",
                version: "v1",
                title: "A finding failed to parse while checking relationship targets",
                status: 400,
                exit_code: 2,
            },
            Self::SubtypeOfCycle { .. } => ProblemMeta {
                slug: "subtype-of-cycle",
                version: "v1",
                title: "subtype_of graph contains a cycle",
                status: 422,
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
            Self::QueueTopicMismatch { .. } => ProblemMeta {
                slug: "queue-topic-mismatch",
                version: "v1",
                title: "Suggestion queue belongs to a different topic",
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
            Self::RegistryFetch { .. } => ProblemMeta {
                slug: "registry-fetch-failed",
                version: "v1",
                title: "Failed to fetch from the ontology registry source",
                status: 502,
                exit_code: 1,
            },
            Self::RegistryIndexInvalid { .. } => ProblemMeta {
                slug: "registry-index-invalid",
                version: "v1",
                title: "Ontology registry index is malformed",
                status: 502,
                exit_code: 1,
            },
            Self::OntologyNotInRegistry { .. } => ProblemMeta {
                slug: "ontology-not-in-registry",
                version: "v1",
                title: "Ontology id has no registry index entry",
                status: 404,
                exit_code: 1,
            },
            Self::IndexPinMismatch { .. } => ProblemMeta {
                slug: "index-pin-mismatch",
                version: "v1",
                title: "Registry index sha256 no longer matches the pinned value",
                status: 409,
                exit_code: 1,
            },
            Self::ChecksumMismatch { .. } => ProblemMeta {
                slug: "ontology-checksum-mismatch",
                version: "v1",
                title: "Fetched ontology file does not match its pinned sha256",
                status: 422,
                exit_code: 1,
            },
            Self::UnsafeIndexPath { .. } => ProblemMeta {
                slug: "unsafe-index-path",
                version: "v1",
                title: "Registry index names an unsafe file path",
                status: 422,
                exit_code: 1,
            },
            Self::MalformedOntologyId { .. } => ProblemMeta {
                slug: "malformed-ontology-id",
                version: "v1",
                title: "Registry index declares a malformed ontology id",
                status: 422,
                exit_code: 1,
            },
            Self::ConfigMalformed { .. } => ProblemMeta {
                slug: "config-malformed",
                version: "v1",
                title: "Harness config's .ontologies field is not an array",
                status: 422,
                exit_code: 1,
            },
            Self::NoEntityTypesFound { .. } => ProblemMeta {
                slug: "no-entity-types-found",
                version: "v1",
                title: "Topic's ontology map carries no typed entities to draft from",
                status: 422,
                exit_code: 1,
            },
            Self::NoClustersFound { .. } => ProblemMeta {
                slug: "no-clusters-found",
                version: "v1",
                title: "Expansion-candidates file carries no clusters to draft from",
                status: 422,
                exit_code: 1,
            },
            Self::VersionNotSemver { .. } => ProblemMeta {
                slug: "version-not-semver",
                version: "v1",
                title: "Version is not well-formed semver",
                status: 422,
                exit_code: 2,
            },
            Self::VersionMissing { .. } => ProblemMeta {
                slug: "version-missing",
                version: "v1",
                title: "File has no .version field",
                status: 422,
                exit_code: 2,
            },
            Self::VersionUnchanged { .. } => ProblemMeta {
                slug: "version-unchanged",
                version: "v1",
                title: "New version equals the current version",
                status: 422,
                exit_code: 2,
            },
            Self::PackNotFound { .. } => ProblemMeta {
                slug: "pack-not-found",
                version: "v1",
                title: "Pack component not found",
                status: 404,
                exit_code: 2,
            },
            Self::PackAmbiguous { .. } => ProblemMeta {
                slug: "pack-ambiguous",
                version: "v1",
                title: "Pack component name is ambiguous",
                status: 422,
                exit_code: 2,
            },
            Self::PackFileMissing { .. } => ProblemMeta {
                slug: "pack-file-missing",
                version: "v1",
                title: "Pack is missing a required file or section",
                status: 422,
                exit_code: 2,
            },
            Self::PackVersionInvalid { .. } => ProblemMeta {
                slug: "pack-version-invalid",
                version: "v1",
                title: "Pack's declared version is not well-formed semver",
                status: 422,
                exit_code: 2,
            },
            Self::PackAheadOfRelease { .. } => ProblemMeta {
                slug: "pack-ahead-of-release",
                version: "v1",
                title: "Pack's current version is ahead of the release being cut",
                status: 409,
                exit_code: 2,
            },
            Self::ChangelogAnchorMissing { .. } => ProblemMeta {
                slug: "changelog-anchor-missing",
                version: "v1",
                title: "CHANGELOG has no anchor to insert the new version under",
                status: 422,
                exit_code: 2,
            },
            Self::VerificationFailed { .. } => ProblemMeta {
                slug: "verification-failed",
                version: "v1",
                title: "Post-write self-verification found an unexpected file",
                status: 500,
                exit_code: 2,
            },
        }
    }
}

/// The `(suggested_fix, code_action)` pair for every variant that carries
/// its own static remediation text (everything except the delegated
/// `Ontology`/`Embed` variants and the IO-classified variants, which
/// `to_problem` handles separately).
// One arm per error variant, each a flat struct literal — length is
// inherent to the variant count, not a complexity signal.
#[allow(clippy::too_many_lines)]
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
        MifRhError::QueueTopicMismatch { .. } => (
            SuggestedFix::new(
                "Point the upsert at reports/_meta/suggestions/<topic>.json for the topic \
                 being reviewed, or remove the stray queue file that was copied or renamed \
                 across topics.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Use the topic's own suggestion queue path",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::JsonSerialize { .. } => (
            SuggestedFix::new(
                "This indicates a bug in mif-rh: a value could not be serialized to JSON. \
                 Report it upstream with the record that triggered it; no caller-side fix \
                 exists.",
                Applicability::Unspecified,
            ),
            CodeAction::new(
                "Report the serialization bug",
                "quickfix",
                Applicability::Unspecified,
            ),
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
        MifRhError::RegistryFetch { .. } => (
            SuggestedFix::new(
                "Check network access to the ontology source (or, for a local directory \
                 source, that the path exists), then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Check the ontology source is reachable",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::RegistryIndexInvalid { .. } => (
            SuggestedFix::new(
                "The registry source's index.json is not valid — fix it upstream, or point \
                 MIF_ONTOLOGY_SOURCE at a known-good mirror, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix or repoint the registry index",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::OntologyNotInRegistry { .. } => (
            SuggestedFix::new(
                "Author the ontology from your research and contribute it upstream with the \
                 `ontology author` subcommand, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Author the missing ontology",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::IndexPinMismatch { .. } => (
            SuggestedFix::new(
                "Confirm the registry source is trustworthy, then clear index_sha256 in \
                 ontologies.lock.json to re-pin deliberately, or investigate why the trust \
                 root moved.",
                Applicability::Unspecified,
            ),
            CodeAction::new(
                "Investigate the moved trust root",
                "quickfix",
                Applicability::Unspecified,
            ),
        ),
        MifRhError::ChecksumMismatch { .. } => (
            SuggestedFix::new(
                "The fetched ontology does not match the pinned registry hash. Do not vendor \
                 it; investigate the registry source for tampering or corruption.",
                Applicability::Unspecified,
            ),
            CodeAction::new(
                "Investigate the checksum mismatch",
                "quickfix",
                Applicability::Unspecified,
            ),
        ),
        MifRhError::NoEntityTypesFound { .. } => (
            SuggestedFix::new(
                "Run /ontology-review on the topic first so its findings get typed, then \
                 retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Review the topic before authoring",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::NoClustersFound { .. } => (
            SuggestedFix::new(
                "Run `mif-rh-cli ontology expansion-candidates` again once more tier-3 misses \
                 have recurred across runs, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Wait for recurring misses before authoring",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::VersionNotSemver { .. } | MifRhError::PackVersionInvalid { .. } => (
            SuggestedFix::new(
                "Correct the version to well-formed X.Y.Z semver, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the malformed version",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::VersionMissing { .. } => (
            SuggestedFix::new(
                "Add a .version field to the file, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Add the missing .version field",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::VersionUnchanged { .. } => (
            SuggestedFix::new(
                "Pass a different version or bump keyword — the requested version matches the \
                 current one, so there is nothing to bump.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Choose a different version",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::PackNotFound { .. } | MifRhError::PackAmbiguous { .. } => (
            SuggestedFix::new(
                "Check the component name against packs/<family>/<name>/ and retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Correct the pack component name",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::PackFileMissing { .. } => (
            SuggestedFix::new(
                "Add the missing file or section the pack's own conventions require \
                 (plugin.json .version, SKILL.md version: frontmatter, or the family doc's \
                 **Version:** row), then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Add the missing pack file/section",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::PackAheadOfRelease { .. } => (
            SuggestedFix::new(
                "Cut a release at or above the pack's current version, or leave this pack out \
                 of this bump.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Reconcile the release version with the pack's version",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::ChangelogAnchorMissing { .. } => (
            SuggestedFix::new(
                "Add an '## [Unreleased]' section to the CHANGELOG, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Add the Unreleased anchor",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::VerificationFailed { .. } => (
            SuggestedFix::new(
                "Inspect the named file directly — the write may have partially applied. This \
                 indicates a bug in the bump logic, not a caller-side fix.",
                Applicability::Unspecified,
            ),
            CodeAction::new(
                "Inspect the file that failed verification",
                "quickfix",
                Applicability::Unspecified,
            ),
        ),
        MifRhError::ConfigMalformed { .. } => (
            SuggestedFix::new(
                "Fix harness.config.json so its .ontologies field is an array, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the .ontologies field's shape",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::UnsafeIndexPath { .. } | MifRhError::MalformedOntologyId { .. } => (
            SuggestedFix::new(
                "The registry index entry is malformed or unsafe — fix it upstream in the \
                 canonical registry before retrying.",
                Applicability::Unspecified,
            ),
            CodeAction::new(
                "Fix the malformed registry entry",
                "quickfix",
                Applicability::Unspecified,
            ),
        ),
        MifRhError::SchemaCompilation { .. } => (
            SuggestedFix::new(
                "Fix the schema (or its $ref dependency) so it is valid JSON Schema, then \
                 retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the malformed schema",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::RefSchemaMissingId { .. } => (
            SuggestedFix::new(
                "Add a $id to the dependency schema, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Add the missing $id",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::SchemaValidationFailed { .. } => (
            SuggestedFix::new(
                "Fix the report to conform to its schema (see the listed validation errors), \
                 then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the schema violations",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::InvalidToggleValue { .. } => (
            SuggestedFix::new(
                "Pass one of the allowed values, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Correct the toggle value",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::EmptySourceContent => (
            SuggestedFix::new(
                "Provide content via --content-file, --content, or stdin.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Provide source content",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::PackNotDeclared { .. } => (
            SuggestedFix::new(
                "Declare the pack in the manifest's packs[] array, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Declare the pack first",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::NoFindingsFound { .. } => (
            SuggestedFix::new(
                "Point at a directory containing finding JSON files, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Point at a non-empty findings directory",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::NoSurvivingFindings { .. } => (
            SuggestedFix::new(
                "Nothing to do until at least one finding survives falsification.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Wait for a surviving finding",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::ArtifactNotPublishable { .. } => (
            SuggestedFix::new(
                "Ensure at least one surviving finding carries a citation before synthesizing.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Add a citation to a surviving finding",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::MissingProvenance { .. } => (
            SuggestedFix::new(
                "Add a provenance block to every listed finding before retrying the import.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Add the missing provenance blocks",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::ReconcileEnvironmentBroken { .. } => (
            SuggestedFix::new(
                "Check the schema/$ref files and the jsonschema toolchain — a known-good \
                 sample should always validate.",
                Applicability::Unspecified,
            ),
            CodeAction::new(
                "Diagnose the schema toolchain",
                "quickfix",
                Applicability::Unspecified,
            ),
        ),
        MifRhError::TopicNotRegistered { .. } => (
            SuggestedFix::new(
                "Register the topic in harness.config.json's topics[] before building its README.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Register the topic",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::InvalidConcordance { .. } => (
            SuggestedFix::new(
                "Build the concordance first (scripts/build-concordance.sh) before synthesizing the corpus atlas.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Build the concordance",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::OntologyMapUnusable { topic, .. } => (
            SuggestedFix::new(
                format!(
                    "Regenerate the ontology map: /ontology-review --topic {topic} --enrich, then /resume --topic {topic}."
                ),
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Regenerate the ontology map",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::RelationshipTargetFindingUnparseable { .. } => (
            SuggestedFix::new(
                "Fix the invalid JSON in this finding before re-running the relationship-targets gate.",
                Applicability::MachineApplicable,
            ),
            CodeAction::new(
                "Fix the malformed finding JSON",
                "quickfix",
                Applicability::MachineApplicable,
            ),
        ),
        MifRhError::SubtypeOfCycle { .. } => (
            SuggestedFix::new(
                "Break the subtype_of cycle in the ontology registry — a type cannot (transitively) subtype itself.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Fix the subtype_of cycle",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        MifRhError::FindingIo { .. }
        | MifRhError::Io { .. }
        | MifRhError::LockIo { .. }
        | MifRhError::Ontology(_)
        | MifRhError::Frontmatter(_)
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
            Self::Frontmatter(inner) => inner.to_problem(),
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

#[cfg(test)]
mod tests {
    use super::MifRhError;

    fn io_error() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::NotFound, "not found")
    }

    fn json_error() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("not json").unwrap_err()
    }

    fn yaml_error() -> serde_norway::Error {
        serde_norway::from_str::<serde_json::Value>("- a\n  bad: [unterminated").unwrap_err()
    }

    // One entry per error variant, each a flat struct literal — length is
    // inherent to the variant count, not a complexity signal.
    #[allow(clippy::too_many_lines)]
    fn every_variant() -> Vec<MifRhError> {
        vec![
            MifRhError::FindingIo {
                path: "f.json".to_string(),
                source: io_error(),
            },
            MifRhError::FindingJson {
                path: "f.json".to_string(),
                source: json_error(),
            },
            MifRhError::Io {
                path: "x".to_string(),
                source: io_error(),
            },
            MifRhError::Json {
                path: "x.json".to_string(),
                source: json_error(),
            },
            MifRhError::JsonSerialize {
                path: "x.json".to_string(),
                source: json_error(),
            },
            MifRhError::OntologyPackYaml {
                path: "x.yaml".to_string(),
                source: yaml_error(),
            },
            MifRhError::CatalogMissing {
                path: "catalog.json".to_string(),
            },
            MifRhError::ConfigMissing {
                path: "config.json".to_string(),
            },
            MifRhError::DirectBindingInvalid {
                topic: "t".to_string(),
                id: "o".to_string(),
            },
            MifRhError::Ontology(mif_ontology::OntologyError::Io {
                path: "onto.yaml".to_string(),
                source: io_error(),
            }),
            MifRhError::EntityTypeSchemaInvalid {
                entity_type: "widget".to_string(),
                detail: "bad schema".to_string(),
            },
            MifRhError::Index {
                source: rusqlite::Error::InvalidParameterName("p".to_string()),
            },
            MifRhError::LockIo {
                path: "lock".to_string(),
                source: io_error(),
            },
            MifRhError::LockHeld { holder_pid: 1234 },
            MifRhError::QueueTopicMismatch {
                path: "reports/_meta/suggestions/edu.json".to_string(),
                expected: "edu".to_string(),
                found: "sec".to_string(),
            },
            MifRhError::Embed(mif_embed::EmbedError::NoCacheDir {
                model: "test-model",
            }),
            MifRhError::RegistryFetch {
                registry_source: "https://example.test/ontologies".to_string(),
                detail: "connection refused".to_string(),
            },
            MifRhError::RegistryIndexInvalid {
                registry_source: "https://example.test/ontologies".to_string(),
                detail: "no .ontologies key".to_string(),
            },
            MifRhError::OntologyNotInRegistry {
                id: "clinical-trials".to_string(),
            },
            MifRhError::IndexPinMismatch {
                registry_source: "https://example.test/ontologies".to_string(),
                pinned: "aaa".to_string(),
                got: "bbb".to_string(),
            },
            MifRhError::ChecksumMismatch {
                id: "edu-fixture".to_string(),
                file: "edu-fixture.ontology.yaml".to_string(),
                expected: "aaa".to_string(),
                got: "bbb".to_string(),
            },
            MifRhError::UnsafeIndexPath {
                id: "edu-fixture".to_string(),
                file: "../../etc/passwd".to_string(),
            },
            MifRhError::MalformedOntologyId {
                id: "../etc".to_string(),
            },
            MifRhError::ConfigMalformed {
                path: "harness.config.json".to_string(),
                detail: ".ontologies is a string, not an array".to_string(),
            },
            MifRhError::NoEntityTypesFound {
                topic: "edu".to_string(),
            },
            MifRhError::NoClustersFound {
                path: "clusters.json".to_string(),
            },
            MifRhError::VersionNotSemver {
                value: "1.0".to_string(),
            },
            MifRhError::VersionMissing {
                path: "harness.config.json".to_string(),
            },
            MifRhError::VersionUnchanged {
                value: "1.0.0".to_string(),
            },
            MifRhError::PackNotFound {
                name: "pdf".to_string(),
            },
            MifRhError::PackAmbiguous {
                name: "pdf".to_string(),
            },
            MifRhError::PackFileMissing {
                name: "pdf".to_string(),
                path: "packs/x/pdf/.claude-plugin/plugin.json".to_string(),
            },
            MifRhError::PackVersionInvalid {
                name: "pdf".to_string(),
                path: "packs/x/pdf/.claude-plugin/plugin.json".to_string(),
                value: "not-semver".to_string(),
            },
            MifRhError::PackAheadOfRelease {
                name: "pdf".to_string(),
                pack_version: "2.0.0".to_string(),
                new_version: "1.5.0".to_string(),
            },
            MifRhError::ChangelogAnchorMissing {
                path: "CHANGELOG.md".to_string(),
            },
            MifRhError::VerificationFailed {
                path: "harness.config.json".to_string(),
            },
            MifRhError::Frontmatter(mif_frontmatter::FrontmatterError::MissingFrontmatter),
            MifRhError::SchemaCompilation {
                path: "findings.schema.json".to_string(),
                detail: "not valid JSON".to_string(),
            },
            MifRhError::RefSchemaMissingId {
                path: "entity-reference.schema.json".to_string(),
            },
            MifRhError::SchemaValidationFailed {
                path: "reports/x/findings/1.md".to_string(),
                schema_path: "schemas/findings.schema.json".to_string(),
                detail: "missing required field 'title'".to_string(),
            },
            MifRhError::InvalidToggleValue {
                field: "primarySurface".to_string(),
                value: "bogus".to_string(),
                allowed: "reports|docs|auto".to_string(),
            },
            MifRhError::EmptySourceContent,
            MifRhError::PackNotDeclared {
                name: "pdf".to_string(),
                path: "harness.config.json".to_string(),
            },
            MifRhError::NoFindingsFound {
                path: "reports/x/findings".to_string(),
            },
            MifRhError::NoSurvivingFindings {
                path: "reports/x/findings".to_string(),
            },
            MifRhError::ArtifactNotPublishable {
                path: "reports/x/findings".to_string(),
            },
            MifRhError::MissingProvenance {
                count: 1,
                paths: vec!["src/findings/f1.json".to_string()],
            },
            MifRhError::ReconcileEnvironmentBroken {
                sample_path: "schemas/samples/finding.sample.json".to_string(),
            },
            MifRhError::TopicNotRegistered {
                topic: "x".to_string(),
                config_path: "harness.config.json".to_string(),
            },
            MifRhError::InvalidConcordance {
                path: "reports/concordance.json".to_string(),
            },
            MifRhError::OntologyMapUnusable {
                path: "reports/edu/ontology-map.json".to_string(),
                topic: "edu".to_string(),
                reason: "is missing".to_string(),
            },
            MifRhError::RelationshipTargetFindingUnparseable {
                path: "reports/edu/findings/bad.json".to_string(),
            },
            MifRhError::SubtypeOfCycle {
                entity_type: "a".to_string(),
            },
        ]
    }

    #[test]
    fn every_variant_produces_a_distinct_problem_type() {
        use mif_problem::ToProblem;

        let problem_types: Vec<String> = every_variant()
            .iter()
            .map(|e| e.to_problem().problem_type)
            .collect();
        let mut deduped = problem_types.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            problem_types.len(),
            deduped.len(),
            "expected every variant to produce a distinct problem_type: {problem_types:?}"
        );
    }

    #[test]
    fn every_variant_has_a_display_message() {
        for error in every_variant() {
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn io_classified_variants_delegate_status_to_classify_io_error() {
        use mif_problem::ToProblem;

        let not_found = MifRhError::Io {
            path: "missing".to_string(),
            source: io_error(),
        };
        // classify_io_error maps NotFound to 404, overriding meta()'s generic 500.
        assert_eq!(not_found.to_problem().status, 404);
    }

    #[test]
    fn delegated_variants_forward_to_the_inner_error() {
        use mif_problem::ToProblem;

        let ontology_problem = MifRhError::Ontology(mif_ontology::OntologyError::Io {
            path: "onto.yaml".to_string(),
            source: io_error(),
        })
        .to_problem();
        let inner_problem = mif_ontology::OntologyError::Io {
            path: "onto.yaml".to_string(),
            source: io_error(),
        }
        .to_problem();
        assert_eq!(ontology_problem.problem_type, inner_problem.problem_type);

        let embed_problem = MifRhError::Embed(mif_embed::EmbedError::NoCacheDir {
            model: "test-model",
        })
        .to_problem();
        let inner_embed_problem = mif_embed::EmbedError::NoCacheDir {
            model: "test-model",
        }
        .to_problem();
        assert_eq!(embed_problem.problem_type, inner_embed_problem.problem_type);
    }

    #[test]
    fn exit_codes_match_the_documented_scheme() {
        use mif_problem::ToProblem;

        assert_eq!(
            MifRhError::CatalogMissing {
                path: "c".to_string()
            }
            .to_problem()
            .exit_code,
            Some(3)
        );
        assert_eq!(
            MifRhError::LockHeld { holder_pid: 1 }
                .to_problem()
                .exit_code,
            Some(2)
        );
        assert_eq!(
            MifRhError::DirectBindingInvalid {
                topic: "t".to_string(),
                id: "o".to_string(),
            }
            .to_problem()
            .exit_code,
            Some(1)
        );
    }

    #[test]
    fn rusqlite_error_converts_into_the_index_variant() {
        let source = rusqlite::Error::InvalidParameterName("p".to_string());
        let error: MifRhError = source.into();
        assert!(matches!(error, MifRhError::Index { .. }));
    }
}
