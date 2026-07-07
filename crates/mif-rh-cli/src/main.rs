//! Command-line interface for [`mif_rh`], the compiled ontology
//! resolution/review engine for research-harness-template (rht) corpora.
//!
//! `resolve`/`review` are drop-in replacements for rht's own
//! `scripts/resolve-ontology.sh`/`scripts/ontology-review.sh`: same flag
//! shapes, same `ontology-map.json`/`--followup` backlog output. A CLI
//! naturally writes to stdout/stderr; this binary exempts itself from the
//! workspace's `print_stdout`/`print_stderr` lints for that reason (see
//! `mif-cli`'s own `CLAUDE.md` note).
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mif_problem::{OutputFormat, ToProblem};

const DEFAULT_CATALOG: &str = ".claude/enabled-packs.json";
const DEFAULT_CONFIG: &str = "harness.config.json";
const DEFAULT_REPORTS_DIR: &str = "reports";
const ONTOLOGY_SOURCE_ENV: &str = "MIF_ONTOLOGY_SOURCE";

#[derive(Parser)]
#[command(
    name = "mif-rh-cli",
    version,
    about = "CLI for the mif-rh research-harness ontology engine"
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
    /// Resolve one finding against its topic's bound ontologies.
    Resolve {
        /// Path to the finding JSON file.
        finding: PathBuf,
        /// The finding's topic. If omitted, derived from `finding`'s path
        /// (`reports/<topic>/...`).
        #[arg(long)]
        topic: Option<String>,
        /// Path to the ontology catalog. Defaults to
        /// `.claude/enabled-packs.json`.
        #[arg(long)]
        catalog: Option<PathBuf>,
        /// Path to the harness config. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Path to write the updated `ontology-map.json` record to. If
        /// omitted and the topic's `reports/<topic>/` directory exists,
        /// defaults to `reports/<topic>/ontology-map.json`; otherwise no
        /// map is written.
        #[arg(long)]
        map: Option<PathBuf>,
        /// Base directory ontology catalog `source` paths resolve against.
        /// Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Rebuild `ontology-map.json` for one or more topics and aggregate
    /// coverage.
    Review {
        /// Topic to review. Repeatable. Defaults to every configured topic.
        #[arg(long)]
        topic: Vec<String>,
        /// Fail (`--strict`) only on invalid/unresolved mappings, never on
        /// discovery-only/untyped findings alone.
        #[arg(long)]
        strict: bool,
        /// Root `reports/` directory. Defaults to `reports`.
        #[arg(long)]
        reports_dir: Option<PathBuf>,
        /// Path to the harness config. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Path to the ontology catalog. Defaults to
        /// `.claude/enabled-packs.json`.
        #[arg(long)]
        catalog: Option<PathBuf>,
        /// Path to write a `--followup` backlog of findings that still
        /// need attention.
        #[arg(long)]
        followup: Option<PathBuf>,
        /// Base directory ontology catalog `source` paths resolve against.
        /// Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Path to rht's `check-relationship-targets.sh`, run once,
        /// corpus-wide, after classification. Defaults to
        /// `<root>/scripts/check-relationship-targets.sh` if that file
        /// exists; otherwise the check is skipped. Unix-only: the script is
        /// spawned directly and relies on its `#!` shebang, which Windows
        /// does not honor.
        #[arg(long)]
        relationship_script: Option<PathBuf>,
        /// Rebuild the corpus-wide search index (every topic in `config`,
        /// not just `--topic`) after classification, for `mif-rh-mcp`'s
        /// `search`/`find_similar` tools. Off by default: index building
        /// re-embeds every finding and is far more expensive than
        /// classification alone.
        #[arg(long)]
        build_index: bool,
        /// Path to the search index database. Defaults to
        /// `<reports-dir>/_meta/search-index.sqlite`.
        #[arg(long)]
        index: Option<PathBuf>,
        /// After classification, write tier-annotated entity-type
        /// suggestions for this review's not-durably-stamped findings to
        /// `<reports-dir>/_meta/suggestions/<topic>.json` (preserving any
        /// confirmed/rejected verdicts), and record tier-3 misses in the
        /// index for `expansion-candidates`. Off by default: suggesting
        /// re-embeds findings, which the fail-closed classification path
        /// must never pay for.
        #[arg(long)]
        suggest: bool,
        /// Path to the confidence-calibration artifact used by
        /// `--suggest`. Defaults to
        /// `<reports-dir>/_meta/confidence-calibration.json`.
        #[arg(long, requires = "suggest")]
        calibration: Option<PathBuf>,
    },
    /// Suggest candidate entity types for a text or a finding, ranked by
    /// embedding similarity with confidence tiers (MIF ADR-020). Prints a
    /// JSON array of hypotheses; never writes to `reports/`.
    SuggestType {
        /// The text to classify. Omit when using `--finding`.
        #[arg(required_unless_present = "finding", conflicts_with = "finding")]
        text: Option<String>,
        /// Path to a finding JSON file whose indexed text (discovery text,
        /// else its entity's name) is the query.
        #[arg(long)]
        finding: Option<PathBuf>,
        /// The topic whose bound ontologies supply candidate entity types.
        /// Required with TEXT; with `--finding` it may instead derive from
        /// the finding's `reports/<topic>/...` path.
        #[arg(long, required_unless_present = "finding")]
        topic: Option<String>,
        /// Path to the ontology catalog. Defaults to
        /// `.claude/enabled-packs.json`.
        #[arg(long)]
        catalog: Option<PathBuf>,
        /// Path to the harness config. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Base directory ontology catalog `source` paths resolve against.
        /// Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Maximum number of ranked candidates to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Path to the confidence-calibration artifact. Defaults to
        /// `reports/_meta/confidence-calibration.json`; when absent,
        /// conservative built-in thresholds apply and candidates carry
        /// `calibrated: false`.
        #[arg(long)]
        calibration: Option<PathBuf>,
        /// Record the query as a tier-3 miss in the search index when its
        /// best candidate is `trigger_expansion` (or no candidate exists),
        /// feeding `expansion-candidates`. Requires `--finding` (a miss is
        /// a property of a finding, not of ad-hoc text).
        #[arg(long, requires = "finding")]
        record: bool,
        /// Path to the search index database `--record` writes to.
        /// Defaults to `reports/_meta/search-index.sqlite`.
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// Derive the corpus's confidence-calibration artifact from its
    /// stamped findings (`stamped-quantile-v1`, MIF ADR-020 PDD-2).
    Calibrate {
        /// Root `reports/` directory. Defaults to `reports`.
        #[arg(long)]
        reports_dir: Option<PathBuf>,
        /// Path to the harness config. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Path to the ontology catalog. Defaults to
        /// `.claude/enabled-packs.json`.
        #[arg(long)]
        catalog: Option<PathBuf>,
        /// Base directory ontology catalog `source` paths resolve against.
        /// Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Minimum empirical top-1 precision the tier-1 gate must achieve.
        #[arg(long, default_value_t = 0.95)]
        target_precision: f32,
        /// Minimum gold-in-candidates rate above the tier-2 floor.
        #[arg(long, default_value_t = 0.5)]
        tier2_target: f32,
        /// Cap the number of stamped samples used (deterministic,
        /// seed-keyed). Defaults to every stamped finding.
        #[arg(long)]
        sample: Option<usize>,
        /// Seed for the deterministic sample selection.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Where to write the calibration artifact. Defaults to
        /// `<reports-dir>/_meta/confidence-calibration.json`.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Also write the ranked confusable type pairs from the stamped
        /// samples here (`confusions-v1` JSON: per pair the gold type, the
        /// type that took top-1, the count, and representative finding
        /// ids), grounding the `negative_examples` curation MIF ADR-020
        /// mandates. Written before the threshold sweep, so an
        /// uncalibratable corpus still gets its confusion export. Derived
        /// data — regenerate, never commit.
        #[arg(long)]
        confusions: Option<PathBuf>,
    },
    /// Cluster recorded tier-3 misses into ontology-expansion candidates
    /// (recurring, mutually-similar misses across runs — never a single
    /// miss). Prints JSON, or writes it with `--out` for
    /// `author-ontology.sh --from-clusters`.
    ExpansionCandidates {
        /// Path to the search index database holding recorded misses.
        /// Defaults to `reports/_meta/search-index.sqlite`.
        #[arg(long)]
        index: Option<PathBuf>,
        /// Path to the confidence-calibration artifact carrying the
        /// clustering knobs. Defaults to
        /// `reports/_meta/confidence-calibration.json`.
        #[arg(long)]
        calibration: Option<PathBuf>,
        /// Write the clusters JSON here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// On-demand ontology vendoring (rht ADR-0012): fetch, catalog, and
    /// verify domain ontology packs from the canonical registry — a
    /// compiled replacement for `fetch-ontology.sh`, the ontology-catalog
    /// section of `sync-packs.sh`, `check-ontology-lock.sh`, and
    /// `sync-registry-ontologies.sh`.
    Ontology {
        #[command(subcommand)]
        action: OntologyCommand,
    },
    /// Harness-native release/versioning tooling (rht Category B, Story
    /// #298): the compiled replacement for `goal-version.sh`,
    /// `bump-version.sh`, and `check-version-bump.sh` (ADR-0010).
    Harness {
        #[command(subcommand)]
        action: HarnessCommand,
    },
}

#[derive(Subcommand)]
enum OntologyCommand {
    /// Vendor one or more ontologies (and their `extends` closure) from the
    /// registry, sha256-verified and pinned in `ontologies.lock.json`.
    Fetch {
        /// Ontology ids to fetch. Mutually exclusive with `--all-enabled`.
        ids: Vec<String>,
        /// Fetch every ontology enabled in `harness.config.json`, instead
        /// of the ids given positionally.
        #[arg(long, conflicts_with = "ids")]
        all_enabled: bool,
        /// Repo root `packs/ontologies/`/`schemas/ontologies/`/
        /// `ontologies.lock.json` resolve against. Defaults to the current
        /// directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Path to the harness config (read for `--all-enabled`). Defaults
        /// to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Override the registry source (a local directory or an http(s)
        /// base URL). Defaults to the `MIF_ONTOLOGY_SOURCE` env var, then
        /// `<root>/.ontologies.source`, then the canonical registry.
        #[arg(long)]
        source: Option<String>,
    },
    /// Rebuild the ontology-catalog section of the catalog sidecar
    /// (`.claude/enabled-packs.json`'s `.ontologies` key) from committed
    /// base layers plus every vendored, enabled ontology.
    Sync {
        /// Repo root ontology directories resolve against. Defaults to the
        /// current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Path to the harness config. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Path to the catalog sidecar to update. Defaults to
        /// `.claude/enabled-packs.json`.
        #[arg(long)]
        catalog: Option<PathBuf>,
    },
    /// Prove every vendored domain ontology matches
    /// `ontologies.lock.json`'s pin (coverage + integrity), fail-closed.
    LockCheck {
        /// Repo root ontology directories and the lock file resolve
        /// against. Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Path to the harness config. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Discover domain ontologies newly published to the registry, add
    /// each (enabled by default) to `harness.config.json`, then vendor and
    /// catalog everything currently enabled.
    SyncRegistry {
        /// Repo root ontology directories resolve against. Defaults to the
        /// current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Path to the harness config. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Path to the catalog sidecar to update. Defaults to
        /// `.claude/enabled-packs.json`.
        #[arg(long)]
        catalog: Option<PathBuf>,
        /// Override the registry source. See `ontology fetch --source`.
        #[arg(long)]
        source: Option<String>,
    },
    /// Draft a new domain ontology YAML from research: entity types a
    /// topic's findings actually used, or clusters of recurring tier-3
    /// misses. Contributing it upstream (`--open-pr`) stays rht's own
    /// script's job — it orchestrates `git`/`gh`, not `jq`.
    Author {
        /// The new ontology's id.
        new_id: String,
        /// Topic dir under `reports/` to mine observed types from.
        /// Required unless `--from-clusters` is given.
        #[arg(
            required_unless_present = "from_clusters",
            conflicts_with = "from_clusters"
        )]
        topic: Option<String>,
        /// Alternate input: a `mif-rh-cli ontology expansion-candidates
        /// --out` file. One candidate type is drafted per cluster.
        #[arg(long)]
        from_clusters: Option<PathBuf>,
        /// Write the draft here. Defaults to a temp file (path printed).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Root `reports/` directory (read for topic mode). Defaults to
        /// `reports`.
        #[arg(long)]
        reports_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum HarnessCommand {
    /// Print a goal's content-hash identity (`gv-<sha256(...)[:12]>`).
    GoalVersion {
        /// Path to `goal.json`.
        goal: PathBuf,
    },
    /// Change-driven version bump (ADR-0010): the release pointer, the
    /// marketplace catalog, a dated CHANGELOG section, and (with `--pack`)
    /// a component's own plugin/skill/doc stamps.
    BumpVersion {
        /// `patch`, `minor`, `major`, or an explicit `X.Y.Z`.
        spec: String,
        /// Also bump this pack component (repeatable).
        #[arg(long = "pack")]
        packs: Vec<String>,
        /// CHANGELOG date for the new section (`YYYY-MM-DD`). Defaults to
        /// today (UTC).
        #[arg(long)]
        date: Option<String>,
        /// Dry run: report what would change, write nothing.
        #[arg(long)]
        check: bool,
        /// Repo root. Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Enforce ADR-0010's change-driven versioning invariants (the PR-only
    /// CI gate): a changed pack/core-skill must move its own version, and
    /// the release pointer must stay ahead of the last release tag.
    CheckVersionBump {
        /// The base ref to diff against. Defaults to `origin/main`.
        base: Option<String>,
        /// Repo root. Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Project a MIF-L3 report's frontmatter + body into JSON and validate
    /// it against a schema (with `$ref` dependencies). Does not run
    /// citation-integrity — that stays a separate check until it is
    /// migrated in its own story.
    ProjectReport {
        /// Path to the report markdown file.
        report: PathBuf,
        /// Path to the schema to validate against.
        #[arg(long)]
        schema: PathBuf,
        /// A `$ref` dependency schema (repeatable). Each must declare its
        /// own `$id`.
        #[arg(long = "ref")]
        refs: Vec<PathBuf>,
        /// Write the projected JSON here.
        #[arg(long = "json-out")]
        json_out: Option<PathBuf>,
    },
    /// Set `.site.primarySurface` in the harness manifest.
    SiteTogglePrimary {
        /// `reports`, `docs`, or `auto`.
        value: String,
        /// Manifest path. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Set `.site.plugins.<name>` in the harness manifest.
    SiteTogglePlugin {
        /// `llmsTxt`, `mermaid`, `imageZoom`, or `linksValidator`.
        name: String,
        /// `on` or `off`.
        state: String,
        /// Manifest path. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Set `.packs[].enabled` for an already-declared pack.
    PackToggle {
        /// The pack's declared name.
        pack: String,
        /// `on` or `off`.
        state: String,
        /// Manifest path. Defaults to `harness.config.json`.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Compose and validate a MIF source-envelope from a raw ingested
    /// source.
    WrapSource {
        /// The source URL.
        #[arg(long)]
        url: String,
        /// The source's MIME content type.
        #[arg(long = "content-type")]
        content_type: String,
        /// The namespace the source belongs to.
        #[arg(long)]
        namespace: String,
        /// The source's slug.
        #[arg(long)]
        slug: String,
        /// Write the envelope here.
        #[arg(long)]
        out: PathBuf,
        /// The source's title. Defaults to `slug`.
        #[arg(long)]
        title: Option<String>,
        /// Read content from this file.
        #[arg(long = "content-file")]
        content_file: Option<PathBuf>,
        /// Content given directly on the command line.
        #[arg(long)]
        content: Option<String>,
        /// The provenance `sourceType`. Defaults to `agent_inferred`.
        #[arg(long = "source-type")]
        source_type: Option<String>,
        /// Path to `schemas/mif/source-envelope.schema.json`.
        #[arg(long)]
        schema: PathBuf,
        /// A `$ref` dependency schema (repeatable). Each must declare its
        /// own `$id`.
        #[arg(long = "ref")]
        refs: Vec<PathBuf>,
    },
    /// Build the MIF-native knowledge graph from a findings directory.
    BuildGraph {
        /// Directory of finding JSON files.
        findings_dir: PathBuf,
        /// Write the graph here. Defaults to `<findings-dir>/../knowledge-graph.json`.
        out: Option<PathBuf>,
    },
    /// Build the flat research index from a findings directory, folding in
    /// the goal-version membership mirror.
    BuildIndex {
        /// Directory of finding JSON files.
        findings_dir: PathBuf,
        /// Write the index here. Defaults to `<findings-dir>/../research-index.json`.
        out: Option<PathBuf>,
    },
    /// Resolve a goal version's deterministic scope over a topic's
    /// findings, writing the authoritative members file.
    ResolveMembership {
        /// The topic name (`reports/<topic>/`).
        topic: String,
        /// The goal version. Defaults to the content hash of
        /// `reports/<topic>/goal.json`.
        version: Option<String>,
        /// Repo root. Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Print a knowledge-graph JSON file's node/edge counts as `NODES
    /// EDGES` (for build-graph-viz.sh's HTML header).
    GraphStats {
        /// Path to a `knowledge-graph.json` file.
        graph: PathBuf,
    },
    /// Render one typed Artifact to an output channel (`report`, `blog`,
    /// or `book`). Composition only — write-then-validate (`report`'s L3
    /// conformance check) stays the bash wrapper's job, via
    /// `mif-project.sh`.
    RenderArtifact {
        /// Path to the `artifact.json` file.
        artifact: PathBuf,
        /// `report`, `blog`, or `book`.
        channel: String,
        /// Write the rendered markdown here.
        out: PathBuf,
        /// The output file's slug (its basename, minus `.md`).
        #[arg(long)]
        slug: String,
        /// The output file's repo-root-relative path.
        #[arg(long)]
        slugpath: String,
        /// The RFC 3339 `created` timestamp.
        #[arg(long)]
        created: String,
        /// This revision's version number.
        #[arg(long)]
        version: u64,
        /// A `verification.json` falsification verdict to fold into the
        /// `report` channel's frontmatter.
        #[arg(long)]
        verification: Option<PathBuf>,
    },
    /// Synthesize an Artifact from a findings directory's surviving
    /// (non-falsified) findings.
    SynthesizeArtifact {
        /// Directory of finding JSON files.
        findings_dir: PathBuf,
        /// The artifact's genre. Defaults to `general`.
        genre: Option<String>,
        /// Write the artifact here. Defaults to `<findings-dir>/../artifact.json`.
        out: Option<PathBuf>,
    },
    /// Validate and import a corpus's findings into a topic, refusing
    /// anything that fails schema validation or lacks a provenance block.
    ImportCorpus {
        /// Directory of source finding JSON files to import.
        src_findings_dir: PathBuf,
        /// Destination findings directory.
        dest_dir: PathBuf,
        /// The topic id to register the corpus under.
        topic: String,
        /// Path to `schemas/findings.schema.json`.
        #[arg(long)]
        schema: PathBuf,
        /// A `$ref` dependency schema (repeatable). Each must declare its
        /// own `$id`.
        #[arg(long = "ref")]
        refs: Vec<PathBuf>,
        /// `harness.config.json`, to register the topic in.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Build the cross-topic concordance (the ontological spine) from
    /// every topic's findings.
    BuildConcordance {
        /// The reports root directory (each subdirectory with a
        /// `findings/` is a topic).
        reports_dir: PathBuf,
        /// Write the concordance here. Defaults to `<reports-dir>/concordance.json`.
        out: Option<PathBuf>,
    },
    /// Derive a durable session checkpoint (`state.json`) purely from
    /// disk and print the remaining-work plan.
    ReconcileSession {
        /// The topic's reports directory (`reports/<topic>`).
        topic_reports_dir: PathBuf,
        /// Path to `schemas/findings.schema.json`.
        #[arg(long)]
        schema: PathBuf,
        /// A `$ref` dependency schema (repeatable). Each must declare its
        /// own `$id`.
        #[arg(long = "ref")]
        refs: Vec<PathBuf>,
        /// A known-good sample finding, validated first as an
        /// environment sanity check.
        #[arg(long)]
        sample: PathBuf,
    },
    /// Compute a topic README's metadata rollup, emitted as
    /// `source`-able shell variable assignments.
    TopicMetadata {
        /// The registered topic id.
        topic: String,
        /// Path to `harness.config.json`.
        #[arg(long)]
        config: PathBuf,
        /// The topic's findings directory.
        #[arg(long)]
        findings: PathBuf,
        /// The topic's `goal.json` (treated as empty if missing/unreadable).
        #[arg(long)]
        goal: PathBuf,
    },
}

/// This binary has no failure modes of its own beyond what [`mif_rh`]
/// already reports — every fallible operation below delegates straight to
/// `mif_rh::MifRhError`, so there is no separate CLI-local error enum to
/// keep in sync with it.
type CliError = mif_rh::MifRhError;

/// A subcommand's successful outcome: the message to print, and the exit
/// code to report — distinct from `Err`, since e.g. an invalid/unresolved
/// classification is still a successfully *produced* record, but must
/// still exit non-zero, matching rht's own bash exit-code contract.
struct Outcome {
    message: String,
    exit_code: u8,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = OutputFormat::select(cli.format.as_deref(), std::io::stderr().is_terminal());
    match run(&cli.command) {
        Ok(outcome) => {
            println!("{}", outcome.message);
            ExitCode::from(outcome.exit_code)
        },
        Err(error) => {
            eprintln!("{}", error.render(format));
            ExitCode::from(error.to_problem().exit_code.unwrap_or(1))
        },
    }
}

fn run(command: &Command) -> Result<Outcome, CliError> {
    match command {
        Command::Resolve {
            finding,
            topic,
            catalog,
            config,
            map,
            root,
        } => resolve(
            finding,
            topic.as_deref(),
            catalog.as_deref(),
            config.as_deref(),
            map.as_deref(),
            root.as_deref(),
        ),
        Command::Review {
            topic,
            strict,
            reports_dir,
            config,
            catalog,
            followup,
            root,
            relationship_script,
            build_index,
            index,
            suggest,
            calibration,
        } => review(&ReviewArgs {
            topics: topic,
            strict: *strict,
            reports_dir: reports_dir.as_deref(),
            config: config.as_deref(),
            catalog: catalog.as_deref(),
            followup: followup.as_deref(),
            root: root.as_deref(),
            relationship_script: relationship_script.as_deref(),
            build_index: *build_index,
            index: index.as_deref(),
            suggest: *suggest,
            calibration: calibration.as_deref(),
        }),
        Command::SuggestType {
            text,
            finding,
            topic,
            catalog,
            config,
            root,
            limit,
            calibration,
            record,
            index,
        } => suggest_type_cmd(&SuggestTypeArgs {
            text: text.as_deref(),
            finding: finding.as_deref(),
            topic: topic.as_deref(),
            catalog: catalog.as_deref(),
            config: config.as_deref(),
            root: root.as_deref(),
            limit: *limit,
            calibration: calibration.as_deref(),
            record: *record,
            index: index.as_deref(),
        }),
        Command::Calibrate {
            reports_dir,
            config,
            catalog,
            root,
            target_precision,
            tier2_target,
            sample,
            seed,
            out,
            confusions,
        } => calibrate_cmd(&CalibrateArgs {
            reports_dir: reports_dir.as_deref(),
            config: config.as_deref(),
            catalog: catalog.as_deref(),
            root: root.as_deref(),
            target_precision: *target_precision,
            tier2_target: *tier2_target,
            sample: *sample,
            seed: *seed,
            out: out.as_deref(),
            confusions: confusions.as_deref(),
        }),
        Command::ExpansionCandidates {
            index,
            calibration,
            out,
        } => expansion_candidates_cmd(index.as_deref(), calibration.as_deref(), out.as_deref()),
        Command::Ontology { action } => ontology_cmd(action),
        Command::Harness { action } => harness_cmd(action),
    }
}

/// Resolves an ontology-registry source override: an explicit `--source`
/// flag first, then the `MIF_ONTOLOGY_SOURCE` env var — `mif_rh::vendor`'s
/// own precedence (a `.ontologies.source` marker file, then the canonical
/// default) applies underneath when both are absent.
fn ontology_source_override(explicit: Option<&str>) -> Option<String> {
    explicit
        .map(str::to_string)
        .or_else(|| std::env::var(ONTOLOGY_SOURCE_ENV).ok())
}

fn ontology_cmd(action: &OntologyCommand) -> Result<Outcome, CliError> {
    match action {
        OntologyCommand::Fetch {
            ids,
            all_enabled,
            root,
            config,
            source,
        } => ontology_fetch_cmd(
            ids,
            *all_enabled,
            root.as_deref(),
            config.as_deref(),
            source.as_deref(),
        ),
        OntologyCommand::Sync {
            root,
            config,
            catalog,
        } => ontology_sync_cmd(root.as_deref(), config.as_deref(), catalog.as_deref()),
        OntologyCommand::LockCheck { root, config } => {
            ontology_lock_check_cmd(root.as_deref(), config.as_deref())
        },
        OntologyCommand::SyncRegistry {
            root,
            config,
            catalog,
            source,
        } => ontology_sync_registry_cmd(
            root.as_deref(),
            config.as_deref(),
            catalog.as_deref(),
            source.as_deref(),
        ),
        OntologyCommand::Author {
            new_id,
            topic,
            from_clusters,
            out,
            reports_dir,
        } => ontology_author_cmd(
            new_id,
            topic.as_deref(),
            from_clusters.as_deref(),
            out.as_deref(),
            reports_dir.as_deref(),
        ),
    }
}

fn ontology_fetch_cmd(
    ids: &[String],
    all_enabled: bool,
    root: Option<&Path>,
    config: Option<&Path>,
    source: Option<&str>,
) -> Result<Outcome, CliError> {
    let root = effective_path(root, ".");
    let resolved_source =
        mif_rh::vendor::resolve_source(&root, ontology_source_override(source).as_deref());
    let fetch_ids = if all_enabled {
        let config_path = effective_path(config, DEFAULT_CONFIG);
        enabled_ontology_ids_from_file(&config_path)?
    } else {
        ids.to_vec()
    };
    if fetch_ids.is_empty() {
        return Ok(Outcome {
            message: "ontology fetch: nothing to fetch (no ids given and no enabled ontologies \
                      configured)"
                .to_string(),
            exit_code: 0,
        });
    }
    let report = mif_rh::vendor::fetch(&root, &resolved_source, &fetch_ids)?;
    let message = if report.vendored.is_empty() {
        "ontology fetch: nothing to fetch (all requested layers are committed base layers or \
         already vendored)"
            .to_string()
    } else {
        let names: Vec<String> = report
            .vendored
            .iter()
            .map(|v| format!("{}@{}", v.id, v.version))
            .collect();
        format!(
            "ontology fetch: vendored {} ({}); lock updated at {}",
            report.vendored.len(),
            names.join(", "),
            root.join("ontologies.lock.json").display()
        )
    };
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

fn ontology_sync_cmd(
    root: Option<&Path>,
    config: Option<&Path>,
    catalog: Option<&Path>,
) -> Result<Outcome, CliError> {
    let root = effective_path(root, ".");
    let config_path = effective_path(config, DEFAULT_CONFIG);
    let catalog_path = effective_path(catalog, DEFAULT_CATALOG);
    let report = mif_rh::vendor::sync_catalog(&root, &config_path, &catalog_path)?;
    Ok(Outcome {
        message: format!(
            "ontology sync: cataloged {} ontolog{} -> {}",
            report.cataloged,
            if report.cataloged == 1 { "y" } else { "ies" },
            catalog_path.display()
        ),
        exit_code: 0,
    })
}

fn ontology_lock_check_cmd(
    root: Option<&Path>,
    config: Option<&Path>,
) -> Result<Outcome, CliError> {
    let root = effective_path(root, ".");
    let config_path = effective_path(config, DEFAULT_CONFIG);
    let report = mif_rh::vendor::lock_check(&root, &config_path)?;
    Ok(format_lock_check_outcome(&report))
}

fn ontology_sync_registry_cmd(
    root: Option<&Path>,
    config: Option<&Path>,
    catalog: Option<&Path>,
    source: Option<&str>,
) -> Result<Outcome, CliError> {
    let root = effective_path(root, ".");
    let config_path = effective_path(config, DEFAULT_CONFIG);
    let catalog_path = effective_path(catalog, DEFAULT_CATALOG);
    let resolved_source =
        mif_rh::vendor::resolve_source(&root, ontology_source_override(source).as_deref());
    let report =
        mif_rh::vendor::sync_registry(&root, &config_path, &catalog_path, &resolved_source)?;
    let message = if report.discovered.is_empty() {
        "ontology sync-registry: no new ontologies in the registry".to_string()
    } else {
        format!(
            "ontology sync-registry: discovered and enabled {} new ontology(ies): {}",
            report.discovered.len(),
            report.discovered.join(", ")
        )
    };
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

/// The subset of `mif-rh-cli ontology expansion-candidates --out` output
/// `ontology author --from-clusters` reads (ignores `misses_considered`/
/// `expansion`, which are diagnostic, not drafting input).
#[derive(serde::Deserialize)]
struct ExpansionCandidatesFile {
    clusters: Vec<mif_rh::ExpansionCandidate>,
}

fn ontology_author_cmd(
    new_id: &str,
    topic: Option<&str>,
    from_clusters: Option<&Path>,
    out: Option<&Path>,
    reports_dir: Option<&Path>,
) -> Result<Outcome, CliError> {
    let report = if let Some(clusters_path) = from_clusters {
        let contents =
            std::fs::read_to_string(clusters_path).map_err(|source| mif_rh::MifRhError::Io {
                path: clusters_path.display().to_string(),
                source,
            })?;
        let file: ExpansionCandidatesFile =
            serde_json::from_str(&contents).map_err(|source| mif_rh::MifRhError::Json {
                path: clusters_path.display().to_string(),
                source,
            })?;
        let source_name = clusters_path.file_name().map_or_else(
            || clusters_path.display().to_string(),
            |name| name.to_string_lossy().to_string(),
        );
        mif_rh::draft_from_clusters(&file.clusters, new_id, &source_name)?
    } else {
        // clap's `required_unless_present` guarantees `topic` is present.
        let topic = topic.unwrap_or_default();
        let reports_dir = effective_path(reports_dir, DEFAULT_REPORTS_DIR);
        let map_path = reports_dir.join(topic).join("ontology-map.json");
        let contents =
            std::fs::read_to_string(&map_path).map_err(|source| mif_rh::MifRhError::Io {
                path: map_path.display().to_string(),
                source,
            })?;
        let records: Vec<mif_rh::MapRecord> =
            serde_json::from_str(&contents).map_err(|source| mif_rh::MifRhError::Json {
                path: map_path.display().to_string(),
                source,
            })?;
        mif_rh::draft_from_topic(&records, new_id, topic)?
    };

    let out_path = out.map_or_else(
        || std::env::temp_dir().join(format!("{new_id}.ontology.yaml")),
        Path::to_path_buf,
    );
    std::fs::write(&out_path, &report.yaml).map_err(|source| mif_rh::MifRhError::Io {
        path: out_path.display().to_string(),
        source,
    })?;

    Ok(Outcome {
        message: format!(
            "ontology author: drafted '{new_id}' with {} candidate type(s) -> {}",
            report.type_count,
            out_path.display()
        ),
        exit_code: 0,
    })
}

// One arm per subcommand variant, each a thin dispatch call — length is
// inherent to the variant count, not a complexity signal.
#[allow(clippy::too_many_lines)]
fn harness_cmd(action: &HarnessCommand) -> Result<Outcome, CliError> {
    match action {
        HarnessCommand::GoalVersion { goal } => harness_goal_version_cmd(goal),
        HarnessCommand::BumpVersion {
            spec,
            packs,
            date,
            check,
            root,
        } => harness_bump_version_cmd(spec, packs, date.as_deref(), *check, root.as_deref()),
        HarnessCommand::CheckVersionBump { base, root } => {
            harness_check_version_bump_cmd(base.as_deref(), root.as_deref())
        },
        HarnessCommand::ProjectReport {
            report,
            schema,
            refs,
            json_out,
        } => harness_project_report_cmd(report, schema, refs, json_out.as_deref()),
        HarnessCommand::SiteTogglePrimary { value, config } => {
            harness_site_toggle_primary_cmd(value, config.as_deref())
        },
        HarnessCommand::SiteTogglePlugin {
            name,
            state,
            config,
        } => harness_site_toggle_plugin_cmd(name, state, config.as_deref()),
        HarnessCommand::PackToggle {
            pack,
            state,
            config,
        } => harness_pack_toggle_cmd(pack, state, config.as_deref()),
        HarnessCommand::WrapSource {
            url,
            content_type,
            namespace,
            slug,
            out,
            title,
            content_file,
            content,
            source_type,
            schema,
            refs,
        } => harness_wrap_source_cmd(&WrapSourceArgs {
            url,
            content_type,
            namespace,
            slug,
            out,
            title: title.as_deref(),
            content_file: content_file.as_deref(),
            content: content.as_deref(),
            source_type: source_type.as_deref(),
            schema,
            refs,
        }),
        HarnessCommand::BuildGraph { findings_dir, out } => {
            harness_build_graph_cmd(findings_dir, out.as_deref())
        },
        HarnessCommand::BuildIndex { findings_dir, out } => {
            harness_build_index_cmd(findings_dir, out.as_deref())
        },
        HarnessCommand::ResolveMembership {
            topic,
            version,
            root,
        } => harness_resolve_membership_cmd(topic, version.as_deref(), root.as_deref()),
        HarnessCommand::GraphStats { graph } => harness_graph_stats_cmd(graph),
        HarnessCommand::RenderArtifact {
            artifact,
            channel,
            out,
            slug,
            slugpath,
            created,
            version,
            verification,
        } => harness_render_artifact_cmd(&RenderArtifactArgs {
            artifact,
            channel,
            out,
            slug,
            slugpath,
            created,
            version: *version,
            verification: verification.as_deref(),
        }),
        HarnessCommand::SynthesizeArtifact {
            findings_dir,
            genre,
            out,
        } => harness_synthesize_artifact_cmd(findings_dir, genre.as_deref(), out.as_deref()),
        HarnessCommand::ImportCorpus {
            src_findings_dir,
            dest_dir,
            topic,
            schema,
            refs,
            config,
        } => harness_import_corpus_cmd(&ImportCorpusArgs {
            src_findings_dir,
            dest_dir,
            topic,
            schema,
            refs,
            config: config.as_deref(),
        }),
        HarnessCommand::BuildConcordance { reports_dir, out } => {
            harness_build_concordance_cmd(reports_dir, out.as_deref())
        },
        HarnessCommand::ReconcileSession {
            topic_reports_dir,
            schema,
            refs,
            sample,
        } => harness_reconcile_session_cmd(topic_reports_dir, schema, refs, sample),
        HarnessCommand::TopicMetadata {
            topic,
            config,
            findings,
            goal,
        } => harness_topic_metadata_cmd(topic, config, findings, goal),
    }
}

fn harness_goal_version_cmd(goal_path: &Path) -> Result<Outcome, CliError> {
    let contents = std::fs::read_to_string(goal_path).map_err(|source| mif_rh::MifRhError::Io {
        path: goal_path.display().to_string(),
        source,
    })?;
    let goal: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| mif_rh::MifRhError::Json {
            path: goal_path.display().to_string(),
            source,
        })?;
    Ok(Outcome {
        message: mif_rh::goal_version_id(&goal),
        exit_code: 0,
    })
}

fn harness_bump_version_cmd(
    spec: &str,
    packs: &[String],
    date: Option<&str>,
    check: bool,
    root: Option<&Path>,
) -> Result<Outcome, CliError> {
    let root = effective_path(root, ".");
    let report = mif_rh::bump_version(&mif_rh::BumpOptions {
        root: &root,
        spec,
        packs,
        date,
        check,
    })?;
    let message = if report.applied {
        format!(
            "bump-version: {} -> {} (date {}){}",
            report.old_version,
            report.new_version,
            report.date,
            if report.packs.is_empty() {
                String::new()
            } else {
                format!("; packs: {}", report.packs.join(", "))
            }
        )
    } else {
        format!(
            "bump-version: --check, {} -> {} (date {}) — no files written",
            report.old_version, report.new_version, report.date
        )
    };
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

fn harness_check_version_bump_cmd(
    base: Option<&str>,
    root: Option<&Path>,
) -> Result<Outcome, CliError> {
    let root = effective_path(root, ".");
    let base = base.unwrap_or("origin/main");
    let report = mif_rh::check_version_bump(&root, base)?;

    let mut lines: Vec<String> = report
        .failures
        .iter()
        .map(|failure| match failure {
            mif_rh::VersionGateFailure::PackNotBumped { pack, version } => format!(
                "FAIL: {pack} changed but its plugin.json .version stayed at {version} — bump it"
            ),
            mif_rh::VersionGateFailure::SkillNotBumped { skill, version } => format!(
                "FAIL: {skill} changed but its SKILL.md version stayed at {version} — bump it"
            ),
            mif_rh::VersionGateFailure::PointerNotAhead {
                current,
                last_release,
            } => format!(
                "FAIL: harness.config.json .version ({current}) is not ahead of the last \
                 release (v{last_release}) — someone needs to bump it before the next release"
            ),
            mif_rh::VersionGateFailure::PointerMissing => {
                "FAIL: harness.config.json has no .version".to_string()
            },
        })
        .collect();
    if lines.is_empty() {
        lines.push(format!(
            "check-version-bump: changed components moved their own version, and the release \
             pointer ({}) is ahead of the last release",
            report.pointer_at_head.as_deref().unwrap_or("-")
        ));
    }
    Ok(Outcome {
        message: lines.join("\n"),
        exit_code: u8::from(!report.failures.is_empty()),
    })
}

fn harness_project_report_cmd(
    report: &Path,
    schema: &Path,
    refs: &[PathBuf],
    json_out: Option<&Path>,
) -> Result<Outcome, CliError> {
    let projected = mif_rh::project_report(report, schema, refs)?;
    if let Some(out_path) = json_out {
        let text = serde_json::to_string_pretty(&projected).map_err(|source| {
            mif_rh::MifRhError::JsonSerialize {
                path: out_path.display().to_string(),
                source,
            }
        })?;
        std::fs::write(out_path, format!("{text}\n")).map_err(|source| mif_rh::MifRhError::Io {
            path: out_path.display().to_string(),
            source,
        })?;
    }
    Ok(Outcome {
        message: format!(
            "mif-project: {} projects to a valid MIF L3 finding",
            report.display()
        ),
        exit_code: 0,
    })
}

fn harness_site_toggle_primary_cmd(
    value: &str,
    config: Option<&Path>,
) -> Result<Outcome, CliError> {
    let config_path = effective_path(config, DEFAULT_CONFIG);
    mif_rh::site_toggle_primary(&config_path, value)?;
    Ok(Outcome {
        message: format!(
            "site-toggle: primarySurface -> {value} in {}",
            config_path.display()
        ),
        exit_code: 0,
    })
}

fn harness_site_toggle_plugin_cmd(
    name: &str,
    state: &str,
    config: Option<&Path>,
) -> Result<Outcome, CliError> {
    let enabled = parse_on_off("site-toggle", state)?;
    let config_path = effective_path(config, DEFAULT_CONFIG);
    mif_rh::site_toggle_plugin(&config_path, name, enabled)?;
    Ok(Outcome {
        message: format!(
            "site-toggle: plugin {name} -> enabled={enabled} in {}",
            config_path.display()
        ),
        exit_code: 0,
    })
}

fn harness_pack_toggle_cmd(
    pack: &str,
    state: &str,
    config: Option<&Path>,
) -> Result<Outcome, CliError> {
    let enabled = parse_on_off("pack-toggle", state)?;
    let config_path = effective_path(config, DEFAULT_CONFIG);
    mif_rh::pack_toggle(&config_path, pack, enabled)?;
    Ok(Outcome {
        message: format!(
            "pack-toggle: {pack} -> enabled={enabled} in {}",
            config_path.display()
        ),
        exit_code: 0,
    })
}

/// Parses a bash-style `on|off` toggle state, reporting `caller` in the
/// error message so `site-toggle`/`pack-toggle` each name themselves.
fn parse_on_off(caller: &str, state: &str) -> Result<bool, CliError> {
    match state {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(mif_rh::MifRhError::InvalidToggleValue {
            field: format!("{caller} state"),
            value: other.to_string(),
            allowed: "on|off".to_string(),
        }),
    }
}

/// Bundles [`HarnessCommand::WrapSource`]'s fields for
/// [`harness_wrap_source_cmd`] to stay under this workspace's
/// too-many-arguments threshold.
struct WrapSourceArgs<'a> {
    url: &'a str,
    content_type: &'a str,
    namespace: &'a str,
    slug: &'a str,
    out: &'a Path,
    title: Option<&'a str>,
    content_file: Option<&'a Path>,
    content: Option<&'a str>,
    source_type: Option<&'a str>,
    schema: &'a Path,
    refs: &'a [PathBuf],
}

fn harness_wrap_source_cmd(args: &WrapSourceArgs<'_>) -> Result<Outcome, CliError> {
    let content_text =
        mif_rh::read_source_content(args.content_file, args.content.unwrap_or_default())?;
    let created = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let envelope = mif_rh::wrap_source(
        &mif_rh::WrapSourceInputs {
            url: args.url,
            content_type: args.content_type,
            namespace: args.namespace,
            slug: args.slug,
            title: args.title.unwrap_or_default(),
            content: &content_text,
            source_type: args.source_type.unwrap_or_default(),
            created: &created,
        },
        args.schema,
        args.refs,
    )?;

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent).map_err(|source| mif_rh::MifRhError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&envelope).map_err(|source| {
        mif_rh::MifRhError::JsonSerialize {
            path: args.out.display().to_string(),
            source,
        }
    })?;
    std::fs::write(args.out, format!("{text}\n")).map_err(|source| mif_rh::MifRhError::Io {
        path: args.out.display().to_string(),
        source,
    })?;

    Ok(Outcome {
        message: format!(
            "wrap-source: wrote {} (urn:mif:source:{}:{}, {})",
            args.out.display(),
            args.namespace,
            args.slug,
            args.content_type
        ),
        exit_code: 0,
    })
}

fn harness_build_graph_cmd(findings_dir: &Path, out: Option<&Path>) -> Result<Outcome, CliError> {
    let out_path = out.map_or_else(
        || findings_dir.join("../knowledge-graph.json"),
        Path::to_path_buf,
    );
    let graph = mif_rh::build_graph(findings_dir)?;
    let node_count = graph["nodes"].as_array().map_or(0, Vec::len);
    let edge_count = graph["edges"].as_array().map_or(0, Vec::len);
    let text = serde_json::to_string_pretty(&graph).map_err(|source| {
        mif_rh::MifRhError::JsonSerialize {
            path: out_path.display().to_string(),
            source,
        }
    })?;
    std::fs::write(&out_path, format!("{text}\n")).map_err(|source| mif_rh::MifRhError::Io {
        path: out_path.display().to_string(),
        source,
    })?;
    Ok(Outcome {
        message: format!(
            "build-graph: wrote {} ({node_count} nodes, {edge_count} edges)",
            out_path.display()
        ),
        exit_code: 0,
    })
}

fn harness_build_index_cmd(findings_dir: &Path, out: Option<&Path>) -> Result<Outcome, CliError> {
    let out_path = out.map_or_else(
        || findings_dir.join("../research-index.json"),
        Path::to_path_buf,
    );
    let index = mif_rh::build_index(findings_dir)?;
    let count = index["count"].as_u64().unwrap_or(0);
    let text = serde_json::to_string_pretty(&index).map_err(|source| {
        mif_rh::MifRhError::JsonSerialize {
            path: out_path.display().to_string(),
            source,
        }
    })?;
    std::fs::write(&out_path, format!("{text}\n")).map_err(|source| mif_rh::MifRhError::Io {
        path: out_path.display().to_string(),
        source,
    })?;
    Ok(Outcome {
        message: format!(
            "build-index: wrote {} ({count} findings)",
            out_path.display()
        ),
        exit_code: 0,
    })
}

fn harness_resolve_membership_cmd(
    topic: &str,
    version: Option<&str>,
    root: Option<&Path>,
) -> Result<Outcome, CliError> {
    let root = effective_path(root, ".");
    let topic_dir = root.join("reports").join(topic);
    let config_path = root.join(DEFAULT_CONFIG);

    let version = if let Some(v) = version {
        v.to_string()
    } else {
        let goal_path = topic_dir.join("goal.json");
        let contents =
            std::fs::read_to_string(&goal_path).map_err(|source| mif_rh::MifRhError::Io {
                path: goal_path.display().to_string(),
                source,
            })?;
        let goal: serde_json::Value =
            serde_json::from_str(&contents).map_err(|source| mif_rh::MifRhError::Json {
                path: goal_path.display().to_string(),
                source,
            })?;
        mif_rh::goal_version_id(&goal)
    };
    let generated = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let report = mif_rh::resolve_membership(
        &topic_dir,
        &config_path,
        &version,
        &generated,
        chrono::Utc::now(),
    )?;

    let members = report.members_file["members"]
        .as_array()
        .map_or(0, Vec::len);
    let stale = report.members_file["stale"].as_array().map_or(0, Vec::len);
    let excluded = report.members_file["excluded"]
        .as_array()
        .map_or(0, Vec::len);
    let gaps: Vec<&str> = report.members_file["gap_dimensions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    let gaps_display = if gaps.is_empty() {
        "none".to_string()
    } else {
        gaps.join(", ")
    };

    Ok(Outcome {
        message: format!(
            "wrote {}\n  members: {members} | stale: {stale} | excluded: {excluded} | gap_dimensions: {gaps_display}",
            report.out_path.display()
        ),
        exit_code: 0,
    })
}

fn harness_graph_stats_cmd(graph: &Path) -> Result<Outcome, CliError> {
    let contents = std::fs::read_to_string(graph).map_err(|source| mif_rh::MifRhError::Io {
        path: graph.display().to_string(),
        source,
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| mif_rh::MifRhError::Json {
            path: graph.display().to_string(),
            source,
        })?;
    let nodes = value["nodes"].as_array().map_or(0, Vec::len);
    let edges = value["edges"].as_array().map_or(0, Vec::len);
    Ok(Outcome {
        message: format!("{nodes} {edges}"),
        exit_code: 0,
    })
}

/// Bundles [`HarnessCommand::RenderArtifact`]'s fields for
/// [`harness_render_artifact_cmd`] to stay under this workspace's
/// too-many-arguments threshold.
struct RenderArtifactArgs<'a> {
    artifact: &'a Path,
    channel: &'a str,
    out: &'a Path,
    slug: &'a str,
    slugpath: &'a str,
    created: &'a str,
    version: u64,
    verification: Option<&'a Path>,
}

fn harness_render_artifact_cmd(args: &RenderArtifactArgs<'_>) -> Result<Outcome, CliError> {
    let contents =
        std::fs::read_to_string(args.artifact).map_err(|source| mif_rh::MifRhError::Io {
            path: args.artifact.display().to_string(),
            source,
        })?;
    let artifact: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| mif_rh::MifRhError::Json {
            path: args.artifact.display().to_string(),
            source,
        })?;

    let verification = args
        .verification
        .map(|path| {
            let text = std::fs::read_to_string(path).map_err(|source| mif_rh::MifRhError::Io {
                path: path.display().to_string(),
                source,
            })?;
            serde_json::from_str::<serde_json::Value>(&text).map_err(|source| {
                mif_rh::MifRhError::Json {
                    path: path.display().to_string(),
                    source,
                }
            })
        })
        .transpose()?;

    let rendered = mif_rh::render_artifact(
        &mif_rh::RenderInputs {
            artifact: &artifact,
            slug: args.slug,
            slugpath: args.slugpath,
            created: args.created,
            version: args.version,
            verification: verification.as_ref(),
        },
        args.channel,
    )?;

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent).map_err(|source| mif_rh::MifRhError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(args.out, &rendered).map_err(|source| mif_rh::MifRhError::Io {
        path: args.out.display().to_string(),
        source,
    })?;

    let line_count = rendered.lines().count();
    Ok(Outcome {
        message: format!(
            "render: wrote {} ({}, {line_count} lines) from {}",
            args.out.display(),
            args.channel,
            args.artifact.display()
        ),
        exit_code: 0,
    })
}

fn harness_synthesize_artifact_cmd(
    findings_dir: &Path,
    genre: Option<&str>,
    out: Option<&Path>,
) -> Result<Outcome, CliError> {
    let genre = genre.unwrap_or("general");
    let out_path = out.map_or_else(|| findings_dir.join("../artifact.json"), Path::to_path_buf);
    let artifact = mif_rh::synthesize_artifact(findings_dir, genre)?;
    let section_count = artifact["sections"].as_array().map_or(0, Vec::len);
    let source_count = artifact["sources"].as_array().map_or(0, Vec::len);
    let text = serde_json::to_string_pretty(&artifact).map_err(|source| {
        mif_rh::MifRhError::JsonSerialize {
            path: out_path.display().to_string(),
            source,
        }
    })?;
    std::fs::write(&out_path, format!("{text}\n")).map_err(|source| mif_rh::MifRhError::Io {
        path: out_path.display().to_string(),
        source,
    })?;
    Ok(Outcome {
        message: format!(
            "synthesize: wrote {} (genre={genre}, {section_count} sections, {source_count} sources)",
            out_path.display()
        ),
        exit_code: 0,
    })
}

/// Bundles [`HarnessCommand::ImportCorpus`]'s fields for
/// [`harness_import_corpus_cmd`] to stay under this workspace's
/// too-many-arguments threshold.
struct ImportCorpusArgs<'a> {
    src_findings_dir: &'a Path,
    dest_dir: &'a Path,
    topic: &'a str,
    schema: &'a Path,
    refs: &'a [PathBuf],
    config: Option<&'a Path>,
}

fn harness_import_corpus_cmd(args: &ImportCorpusArgs<'_>) -> Result<Outcome, CliError> {
    let report = mif_rh::import_corpus(
        args.src_findings_dir,
        args.dest_dir,
        args.topic,
        args.config,
        args.schema,
        args.refs,
    )?;
    let mut message = format!(
        "import: imported {} finding(s) into {} (namespace {})",
        report.imported,
        args.dest_dir.display(),
        report.namespace
    );
    if report.topic_registered {
        use std::fmt::Write as _;
        let _ = write!(message, "; registered topic '{}'", args.topic);
    }
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

fn harness_build_concordance_cmd(
    reports_dir: &Path,
    out: Option<&Path>,
) -> Result<Outcome, CliError> {
    let out_path = out.map_or_else(|| reports_dir.join("concordance.json"), Path::to_path_buf);
    let concordance = mif_rh::build_concordance(reports_dir)?;
    let node_count = concordance["nodes"].as_array().map_or(0, Vec::len);
    let edge_count = concordance["edges"].as_array().map_or(0, Vec::len);
    let text = serde_json::to_string_pretty(&concordance).map_err(|source| {
        mif_rh::MifRhError::JsonSerialize {
            path: out_path.display().to_string(),
            source,
        }
    })?;
    std::fs::write(&out_path, format!("{text}\n")).map_err(|source| mif_rh::MifRhError::Io {
        path: out_path.display().to_string(),
        source,
    })?;
    Ok(Outcome {
        message: format!(
            "build-concordance: wrote {} ({node_count} nodes, {edge_count} edges) across topics",
            out_path.display()
        ),
        exit_code: 0,
    })
}

fn harness_reconcile_session_cmd(
    topic_reports_dir: &Path,
    schema: &Path,
    refs: &[PathBuf],
    sample: &Path,
) -> Result<Outcome, CliError> {
    let report = mif_rh::reconcile_session(topic_reports_dir, schema, refs, sample)?;
    let message = if report.plan.is_empty() {
        "nothing to do".to_string()
    } else {
        format!("REMAINING WORK PLAN\n{}", report.plan.join("\n"))
    };
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

fn harness_topic_metadata_cmd(
    topic: &str,
    config: &Path,
    findings: &Path,
    goal: &Path,
) -> Result<Outcome, CliError> {
    let metadata = mif_rh::topic_metadata(topic, config, findings, goal)?;
    Ok(Outcome {
        message: metadata.to_shell_script(),
        exit_code: 0,
    })
}

/// Reads `harness.config.json`'s `.ontologies[]` directly (rather than
/// through [`mif_rh::HarnessConfig`], which deliberately only models
/// `topics[]`) to collect every enabled ontology id for `ontology fetch
/// --all-enabled`.
fn enabled_ontology_ids_from_file(config_path: &Path) -> Result<Vec<String>, CliError> {
    if !config_path.exists() {
        return Err(mif_rh::MifRhError::ConfigMissing {
            path: config_path.display().to_string(),
        });
    }
    let contents =
        std::fs::read_to_string(config_path).map_err(|source| mif_rh::MifRhError::Io {
            path: config_path.display().to_string(),
            source,
        })?;
    let config: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| mif_rh::MifRhError::Json {
            path: config_path.display().to_string(),
            source,
        })?;
    let ids = config
        .get("ontologies")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("enabled")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                })
                .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(ids)
}

/// Formats `check-ontology-lock.sh`'s own per-issue lines plus a final
/// summary, matching its fail-closed exit-code contract (non-zero on any
/// missing pin, un-vendored ontology, or drift).
fn format_lock_check_outcome(report: &mif_rh::LockCheckReport) -> Outcome {
    let mut lines = Vec::new();
    for id in &report.missing_pins {
        lines.push(format!(
            "  MISSING PIN: '{id}' is enabled but absent from the lock — run `ontology fetch {id}`"
        ));
    }
    for id in &report.not_vendored {
        lines.push(format!(
            "  NOT VENDORED: enabled '{id}' is pinned but its file is absent — run `ontology \
             fetch {id}`"
        ));
    }
    for drift in &report.drift {
        lines.push(format!(
            "  DRIFT: '{}' sha256 {} != pinned {} — a vendored ontology was edited locally; \
             change it upstream and re-fetch",
            drift.id, drift.got, drift.pinned
        ));
    }
    if report.ok() {
        lines.push(format!(
            "check-ontology-lock: ok ({} vendored ontolog{} match the lock)",
            report.checked,
            if report.checked == 1 { "y" } else { "ies" }
        ));
    }
    Outcome {
        message: lines.join("\n"),
        exit_code: u8::from(!report.ok()),
    }
}

fn effective_path(given: Option<&Path>, default: &str) -> PathBuf {
    given.map_or_else(|| PathBuf::from(default), Path::to_path_buf)
}

/// Derives a finding's topic from its path (`reports/<topic>/...`), the
/// same convention `resolve-ontology.sh` uses when `--topic` is omitted.
fn topic_from_path(finding: &Path) -> Option<String> {
    let components: Vec<&str> = finding
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let index = components.iter().position(|c| *c == "reports")?;
    components.get(index + 1).map(|s| (*s).to_string())
}

fn resolve(
    finding_path: &Path,
    topic: Option<&str>,
    catalog: Option<&Path>,
    config: Option<&Path>,
    map: Option<&Path>,
    root: Option<&Path>,
) -> Result<Outcome, CliError> {
    let catalog_path = effective_path(catalog, DEFAULT_CATALOG);
    let config_path = effective_path(config, DEFAULT_CONFIG);
    let root = effective_path(root, ".");

    let topic = topic
        .map(str::to_string)
        .or_else(|| topic_from_path(finding_path))
        .unwrap_or_default();

    let finding = mif_rh::Finding::load(finding_path)?;
    let catalog = mif_rh::Catalog::load(&catalog_path)?;
    let config = mif_rh::HarnessConfig::load(&config_path)?;
    let ontology_packs = mif_rh::ontology_pack::load_packs_via_catalog(&catalog, &root)?;

    let ctx = mif_rh::ResolveContext {
        topic: &topic,
        catalog: &catalog,
        config: &config,
        ontology_packs: &ontology_packs,
    };
    let record = mif_rh::resolve_finding(&finding, &ctx)?;

    let map_path = map.map(Path::to_path_buf).or_else(|| {
        let topic_dir = PathBuf::from("reports").join(&topic);
        topic_dir
            .is_dir()
            .then(|| topic_dir.join("ontology-map.json"))
    });
    if let Some(map_path) = &map_path {
        upsert_map_record(map_path, &record)?;
    }

    // `record.valid` is already `true` for every `Discovery`/`Untyped`
    // record (see `discovery_classify`), so it alone captures "ok" — no
    // separate `Basis::Untyped` carve-out is needed.
    let message = format!(
        "{}: {} -> {} (valid={})",
        finding.id,
        record.basis.label(),
        record.resolved_ontology.as_deref().unwrap_or("-"),
        record.valid
    );
    Ok(Outcome {
        message,
        exit_code: u8::from(!record.valid),
    })
}

/// Upserts one record into a per-topic `ontology-map.json`, replacing any
/// existing record for the same `finding_id`, matching
/// `resolve-ontology.sh`'s own `record()` upsert semantics. A corrupt or
/// missing existing map resets to an empty one rather than blocking the
/// upsert.
fn upsert_map_record(
    map_path: &Path,
    record: &mif_rh::MapRecord,
) -> Result<(), mif_rh::MifRhError> {
    let mut records: Vec<mif_rh::MapRecord> = std::fs::read_to_string(map_path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default();
    records.retain(|r| r.finding_id != record.finding_id);
    records.push(record.clone());
    records.sort_by(|a, b| a.finding_id.cmp(&b.finding_id));

    mif_rh::write_json_atomic(map_path, &records)
}

/// Arguments for the `suggest-type` subcommand, bundled to match the
/// `ReviewArgs` convention below.
struct SuggestTypeArgs<'a> {
    text: Option<&'a str>,
    finding: Option<&'a Path>,
    topic: Option<&'a str>,
    catalog: Option<&'a Path>,
    config: Option<&'a Path>,
    root: Option<&'a Path>,
    limit: usize,
    calibration: Option<&'a Path>,
    record: bool,
    index: Option<&'a Path>,
}

const DEFAULT_CALIBRATION: &str = "reports/_meta/confidence-calibration.json";
const DEFAULT_INDEX: &str = "reports/_meta/search-index.sqlite";

/// A run identifier for miss recording: wall-clock seconds plus pid —
/// distinct across real runs (what tier-3 recurrence counts), stable
/// within one invocation.
fn run_id() -> String {
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{epoch_secs}-{}", std::process::id())
}

/// Whether a suggestion outcome is a tier-3 miss worth recording: no
/// candidate at all, or a best candidate in the `trigger_expansion` band.
fn is_expansion_miss(suggestions: &[mif_rh::TypeSuggestion]) -> bool {
    suggestions
        .first()
        .is_none_or(|top| top.tier == mif_ontology::ConfidenceTier::TriggerExpansion)
}

/// Runs `suggest-type`: derives the query text (given directly, or a
/// finding's indexed text), loads the topic's ontology context and the
/// calibration artifact, and prints the tier-annotated candidate list as
/// pretty JSON. Exit code 0 — a hypothesis list, even an empty one, is a
/// successful outcome, and nothing is ever written to `reports/`.
fn suggest_type_cmd(args: &SuggestTypeArgs<'_>) -> Result<Outcome, CliError> {
    let catalog_path = effective_path(args.catalog, DEFAULT_CATALOG);
    let config_path = effective_path(args.config, DEFAULT_CONFIG);
    let root = effective_path(args.root, ".");
    let calibration_path = effective_path(args.calibration, DEFAULT_CALIBRATION);

    // clap guarantees exactly one of text/--finding, and --topic whenever
    // text is given; with --finding the topic may still derive from the
    // finding's reports/<topic>/... path, mirroring `resolve`.
    let (query, topic, finding_id) = if let Some(finding_path) = args.finding {
        let finding = mif_rh::Finding::load(finding_path)?;
        let topic = args
            .topic
            .map(str::to_string)
            .or_else(|| topic_from_path(finding_path))
            .unwrap_or_default();
        (mif_rh::index_text(&finding), topic, Some(finding.id))
    } else {
        (
            args.text.unwrap_or_default().to_string(),
            args.topic.unwrap_or_default().to_string(),
            None,
        )
    };

    let catalog = mif_rh::Catalog::load(&catalog_path)?;
    let config = mif_rh::HarnessConfig::load(&config_path)?;
    let ontology_packs = mif_rh::ontology_pack::load_packs_via_catalog(&catalog, &root)?;
    let ctx = mif_rh::ResolveContext {
        topic: &topic,
        catalog: &catalog,
        config: &config,
        ontology_packs: &ontology_packs,
    };
    let cal = mif_ontology::CalibrationConfig::load_or_default(&calibration_path)
        .map_err(mif_rh::MifRhError::from)?;
    let embedder = mif_embed::Embedder::load()?;
    // Embed the query once; the vector serves both the ranking and — on a
    // tier-3 miss — the recorded miss, with no second forward pass.
    let candidates = mif_rh::suggest::build_candidates(&ctx, &embedder, &cal)?;
    let query_vector = embedder.embed(&query)?;
    let suggestions =
        mif_rh::suggest::suggest_from_candidates(&query_vector, &candidates, &cal, args.limit);

    if args.record && is_expansion_miss(&suggestions) {
        // clap's `requires = "finding"` guarantees the id was captured above.
        if let Some(finding_id) = finding_id {
            let index_path = effective_path(args.index, DEFAULT_INDEX);
            if let Some(parent) = index_path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| mif_rh::MifRhError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
            let index = mif_rh::FindingIndex::open(&index_path)?;
            index.record_miss(&mif_rh::Miss {
                finding_id,
                topic: topic.clone(),
                content: query,
                vector: query_vector,
                run_id: run_id(),
                model: mif_embed::MODEL_ID.to_string(),
            })?;
        }
    }

    let message =
        serde_json::to_string_pretty(&suggestions).map_err(|source| CliError::JsonSerialize {
            path: "<stdout>".to_string(),
            source,
        })?;
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

/// Arguments for the `calibrate` subcommand.
struct CalibrateArgs<'a> {
    reports_dir: Option<&'a Path>,
    config: Option<&'a Path>,
    catalog: Option<&'a Path>,
    root: Option<&'a Path>,
    target_precision: f32,
    tier2_target: f32,
    sample: Option<usize>,
    seed: u64,
    out: Option<&'a Path>,
    confusions: Option<&'a Path>,
}

/// Runs `calibrate`: collects stamped-finding samples across every
/// configured topic, sweeps the threshold grid, and atomically writes the
/// calibration artifact.
fn calibrate_cmd(args: &CalibrateArgs<'_>) -> Result<Outcome, CliError> {
    let reports_dir = effective_path(args.reports_dir, DEFAULT_REPORTS_DIR);
    let config_path = effective_path(args.config, DEFAULT_CONFIG);
    let catalog_path = effective_path(args.catalog, DEFAULT_CATALOG);
    let root = effective_path(args.root, ".");
    let out = args.out.map_or_else(
        || reports_dir.join("_meta/confidence-calibration.json"),
        Path::to_path_buf,
    );

    let catalog = mif_rh::Catalog::load(&catalog_path)?;
    let config = mif_rh::HarnessConfig::load(&config_path)?;
    let ontology_packs = mif_rh::ontology_pack::load_packs_via_catalog(&catalog, &root)?;
    let embedder = mif_embed::Embedder::load()?;

    let opts = mif_rh::CalibrateOptions {
        target_precision: args.target_precision,
        tier2_target: args.tier2_target,
        sample: args.sample,
        seed: args.seed,
    };
    let mut samples = Vec::new();
    for topic in &config.topics {
        let ctx = mif_rh::ResolveContext {
            topic: &topic.id,
            catalog: &catalog,
            config: &config,
            ontology_packs: &ontology_packs,
        };
        samples.extend(mif_rh::collect_topic_samples(
            &reports_dir,
            &ctx,
            &embedder,
        )?);
    }
    let samples = mif_rh::subsample(samples, &opts);

    // The confusion export writes BEFORE the sweep: a corpus that cannot
    // reach the precision target is exactly the corpus whose confusions
    // curation needs to see, so a failing sweep must not take the export
    // down with it.
    let confusions_note = if let Some(confusions_path) = args.confusions {
        let report = mif_rh::confusions(&samples);
        if let Some(parent) = confusions_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| mif_rh::MifRhError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        mif_rh::write_json_atomic(confusions_path, &report)?;
        format!(
            ", {} confusion pair(s) written to {}",
            report.pairs.len(),
            confusions_path.display()
        )
    } else {
        String::new()
    };

    let mut cal = mif_rh::sweep(&samples, &opts, &out)?;
    // Record whether curated negatives were among the candidates that
    // scored the swept samples: only packs resolved for topics actually
    // represented in the final (post-subsample) sample set count — an
    // enabled-but-unbound negatives pack must not claim participation.
    let scored_topics: std::collections::BTreeSet<&str> =
        samples.iter().map(|s| s.topic.as_str()).collect();
    let mut negatives_active = false;
    for topic in &config.topics {
        if !scored_topics.contains(topic.id.as_str()) {
            continue;
        }
        let ctx = mif_rh::ResolveContext {
            topic: &topic.id,
            catalog: &catalog,
            config: &config,
            ontology_packs: &ontology_packs,
        };
        if mif_rh::packs_carry_negatives(mif_rh::build_allowed(&ctx)?) {
            negatives_active = true;
            break;
        }
    }
    cal.negatives_active = negatives_active;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|source| mif_rh::MifRhError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    mif_rh::write_json_atomic(&out, &cal)?;

    let message = format!(
        "calibrate: {} sample(s) -> tier1_floor={:.2} tier1_margin={:.2} tier2_floor={:.2} \
         (method {}, written to {}{confusions_note})",
        samples.len(),
        cal.tier1_floor,
        cal.tier1_margin,
        cal.tier2_floor,
        cal.method.as_deref().unwrap_or("-"),
        out.display()
    );
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

/// Runs `expansion-candidates`: clusters recorded tier-3 misses under the
/// calibration artifact's expansion knobs and emits the candidates as
/// JSON (stdout, or `--out` for `author-ontology.sh --from-clusters`).
fn expansion_candidates_cmd(
    index: Option<&Path>,
    calibration: Option<&Path>,
    out: Option<&Path>,
) -> Result<Outcome, CliError> {
    let index_path = effective_path(index, DEFAULT_INDEX);
    let calibration_path = effective_path(calibration, DEFAULT_CALIBRATION);

    let cal = mif_ontology::CalibrationConfig::load_or_default(&calibration_path)
        .map_err(mif_rh::MifRhError::from)?;
    // A read-only query must not create the index as a side effect: a
    // missing index simply means no misses were ever recorded.
    let misses = if index_path.exists() {
        let idx = mif_rh::FindingIndex::open(&index_path)?;
        idx.misses()?
    } else {
        Vec::new()
    };
    // Vectors from different embedding models share no space; cluster
    // only what the model in use produced.
    let misses: Vec<mif_rh::Miss> = misses
        .into_iter()
        .filter(|m| m.model == mif_embed::MODEL_ID)
        .collect();
    let candidates = mif_rh::expansion_candidates(&misses, &cal.expansion);

    let payload = serde_json::json!({
        "clusters": candidates,
        "misses_considered": misses.len(),
        "expansion": cal.expansion,
    });
    if let Some(out_path) = out {
        mif_rh::write_json_atomic(out_path, &payload)?;
        return Ok(Outcome {
            message: format!(
                "expansion-candidates: {} cluster(s) from {} miss(es) written to {}",
                candidates.len(),
                misses.len(),
                out_path.display()
            ),
            exit_code: 0,
        });
    }
    let message =
        serde_json::to_string_pretty(&payload).map_err(|source| CliError::JsonSerialize {
            path: "<stdout>".to_string(),
            source,
        })?;
    Ok(Outcome {
        message,
        exit_code: 0,
    })
}

/// Arguments for the `review` subcommand, bundled (rather than passed as
/// ~10 positional parameters) to match `mif_rh::ReviewOptions`'s own
/// struct-of-options convention.
struct ReviewArgs<'a> {
    topics: &'a [String],
    strict: bool,
    reports_dir: Option<&'a Path>,
    config: Option<&'a Path>,
    catalog: Option<&'a Path>,
    followup: Option<&'a Path>,
    root: Option<&'a Path>,
    relationship_script: Option<&'a Path>,
    build_index: bool,
    index: Option<&'a Path>,
    suggest: bool,
    calibration: Option<&'a Path>,
}

/// Inputs for `review --suggest`'s post-classification suggestion pass.
struct SuggestPassInputs<'a> {
    backlog: &'a mif_rh::FollowupBacklog,
    catalog: &'a mif_rh::Catalog,
    config: &'a mif_rh::HarnessConfig,
    ontology_packs: &'a std::collections::HashMap<String, mif_rh::OntologyPack>,
    meta_dir: &'a Path,
    index: Option<&'a Path>,
    calibration: Option<&'a Path>,
}

/// Runs the opt-in `--suggest` pass over a review's followup findings:
/// writes tier-annotated suggestion queues under
/// `<meta_dir>/suggestions/<topic>.json` (preserving human verdicts) and
/// records tier-3 misses in the index. Returns the confirmation line.
fn run_suggest_pass(inputs: &SuggestPassInputs<'_>) -> Result<String, CliError> {
    let calibration_path = inputs.calibration.map_or_else(
        || inputs.meta_dir.join("confidence-calibration.json"),
        Path::to_path_buf,
    );
    let cal = mif_ontology::CalibrationConfig::load_or_default(&calibration_path)
        .map_err(mif_rh::MifRhError::from)?;
    let embedder = mif_embed::Embedder::load()?;
    let index_path = inputs.index.map_or_else(
        || inputs.meta_dir.join("search-index.sqlite"),
        Path::to_path_buf,
    );
    let index = mif_rh::FindingIndex::open(&index_path)?;
    let run = run_id();

    let mut topic_ids: Vec<&String> = inputs.backlog.topics.keys().collect();
    topic_ids.sort();

    let (mut total_entries, mut topics_written, mut misses_recorded) = (0_usize, 0_usize, 0_usize);
    for topic_id in topic_ids {
        let ctx = mif_rh::ResolveContext {
            topic: topic_id,
            catalog: inputs.catalog,
            config: inputs.config,
            ontology_packs: inputs.ontology_packs,
        };
        // One embedding pass over this topic's candidate documents,
        // reused for every followup finding below.
        let candidates = mif_rh::suggest::build_candidates(&ctx, &embedder, &cal)?;
        let mut fresh = Vec::new();
        for followup_entry in &inputs.backlog.topics[topic_id] {
            // A "gap" entry's file could not even be parsed by review;
            // skipping it here mirrors that — the followup backlog still
            // carries it for a human.
            let Some(file) = &followup_entry.file else {
                continue;
            };
            let Ok(finding) = mif_rh::Finding::load(Path::new(file)) else {
                continue;
            };
            let query = mif_rh::index_text(&finding);
            if query.is_empty() {
                continue;
            }
            let query_vector = embedder.embed(&query)?;
            let suggestions = mif_rh::suggest::suggest_from_candidates(
                &query_vector,
                &candidates,
                &cal,
                mif_rh::suggest::SUGGESTION_DEPTH,
            );
            if is_expansion_miss(&suggestions) {
                index.record_miss(&mif_rh::Miss {
                    finding_id: finding.id.clone(),
                    topic: topic_id.clone(),
                    vector: query_vector,
                    content: query,
                    run_id: run.clone(),
                    model: mif_embed::MODEL_ID.to_string(),
                })?;
                misses_recorded += 1;
            }
            fresh.push(mif_rh::SuggestionEntry {
                finding_id: finding.id,
                file: followup_entry.file.clone(),
                basis: followup_entry.basis.clone(),
                run_id: run.clone(),
                candidates: suggestions,
                status: mif_rh::queue::STATUS_PENDING.to_string(),
            });
        }
        if fresh.is_empty() {
            continue;
        }
        total_entries += fresh.len();
        topics_written += 1;
        let queue_path = inputs
            .meta_dir
            .join("suggestions")
            .join(format!("{topic_id}.json"));
        mif_rh::upsert_suggestions(&queue_path, topic_id, fresh)?;
    }

    if total_entries == 0 {
        return Ok(format!(
            "ontology-review: no findings needed suggestions; {misses_recorded} miss(es) recorded"
        ));
    }
    Ok(format!(
        "ontology-review: suggestions written to {} ({} finding(s) across {} topic(s); {} \
         miss(es) recorded)",
        inputs.meta_dir.join("suggestions").display(),
        total_entries,
        topics_written,
        misses_recorded,
    ))
}

/// Formats the `TOPIC BOUND FIND STAMPED DISCOVERY UNTYPED INVALID` table
/// `ontology-review.sh` prints once per reviewed topic, byte-for-byte
/// (`printf '%-28s %-22s %6s %8s %10s %8s %9s\n'`), plus its trailing
/// `---` separator before the final summary line.
fn format_topic_table(report: &mif_rh::ReviewReport) -> String {
    use std::fmt::Write as _;

    let mut out = format!(
        "{:<28} {:<22} {:>6} {:>8} {:>10} {:>8} {:>9}\n",
        "TOPIC", "BOUND", "FIND", "STAMPED", "DISCOVERY", "UNTYPED", "INVALID"
    );
    for topic in &report.topics {
        let bound = if topic.bound.is_empty() {
            "(core-only)".to_string()
        } else {
            topic.bound.join(",")
        };
        let bound: String = bound.chars().take(22).collect();
        let _ = writeln!(
            out,
            "{:<28} {:<22} {:>6} {:>8} {:>10} {:>8} {:>9}",
            topic.topic,
            bound,
            topic.total,
            topic.stamped,
            topic.discovery,
            topic.untyped,
            topic.bad
        );
    }
    out.push_str("---");
    out
}

/// Resolves rht's `check-relationship-targets.sh` path: the explicit
/// override if given, otherwise `<root>/scripts/check-relationship-targets.sh`
/// if that file exists, otherwise `None` (skip the check — matches
/// reviewing a corpus with no rht scripts available, e.g. isolated tests).
fn relationship_script_path(given: Option<&Path>, root: &Path) -> Option<PathBuf> {
    given.map(Path::to_path_buf).or_else(|| {
        let candidate = root.join("scripts/check-relationship-targets.sh");
        candidate.is_file().then_some(candidate)
    })
}

fn review(args: &ReviewArgs<'_>) -> Result<Outcome, CliError> {
    let reports_dir = effective_path(args.reports_dir, DEFAULT_REPORTS_DIR);
    let config_path = effective_path(args.config, DEFAULT_CONFIG);
    let catalog_path = effective_path(args.catalog, DEFAULT_CATALOG);
    let root = effective_path(args.root, ".");
    let meta_dir = reports_dir.join("_meta");
    std::fs::create_dir_all(&meta_dir).map_err(|source| mif_rh::MifRhError::Io {
        path: meta_dir.display().to_string(),
        source,
    })?;

    // Held for the rest of this function; released on drop. The direct fix
    // for two concurrent `review` runs corrupting `ontology-map.json`
    // mid-write (see `mif_rh::ReviewLock`'s own doc comment).
    let _lock = mif_rh::ReviewLock::acquire(&meta_dir.join(".review.lock"))?;

    let catalog = mif_rh::Catalog::load(&catalog_path)?;
    let config = mif_rh::HarnessConfig::load(&config_path)?;
    let ontology_packs = mif_rh::ontology_pack::load_packs_via_catalog(&catalog, &root)?;

    let relationship_script = relationship_script_path(args.relationship_script, &root);
    let topic_ids: Option<Vec<String>> = (!args.topics.is_empty()).then(|| args.topics.to_vec());
    let opts = mif_rh::ReviewOptions {
        topics: topic_ids.as_deref(),
        reports_dir: &reports_dir,
        ontology_packs: &ontology_packs,
        catalog: &catalog,
        config: &config,
        check_relationship_targets_script: relationship_script.as_deref(),
    };

    let (report, backlog) = mif_rh::review(&opts)?;

    let mut message = format_topic_table(&report);

    // Matches `ontology-review.sh`'s own output order exactly: the
    // `--followup` write confirmation prints before the final "---" +
    // summary line, so a caller capturing only the last stdout line (as
    // `verify.sh`'s gate_m12 does) always sees the aggregate summary, not
    // this confirmation.
    if let Some(followup_path) = args.followup {
        use std::fmt::Write as _;

        mif_rh::write_followup(followup_path, &backlog)?;
        message.push('\n');
        let _ = write!(
            message,
            "ontology-review: followup backlog written to {} ({} finding(s) across {} topic(s))",
            followup_path.display(),
            backlog.total_needs_followup,
            backlog.topics.len(),
        );
    }

    // Like the followup confirmation above: prints before the final
    // summary line, which callers treat as the last line of output.
    if args.suggest {
        message.push('\n');
        message.push_str(&run_suggest_pass(&SuggestPassInputs {
            backlog: &backlog,
            catalog: &catalog,
            config: &config,
            ontology_packs: &ontology_packs,
            meta_dir: &meta_dir,
            index: args.index,
            calibration: args.calibration,
        })?);
    }

    message.push('\n');
    message.push_str(&report.summary_line());

    if args.build_index {
        // Always the full corpus (every topic in `config`), never just the
        // topic(s) this run classified — a finding discovered while
        // researching one topic remains a searchable source for every
        // future topic, so a scoped `--topic` review must not narrow the
        // search index down to only what it just reviewed.
        let all_topic_ids: Vec<String> = config.topics.iter().map(|t| t.id.clone()).collect();
        let index_path = args
            .index
            .map_or_else(|| meta_dir.join("search-index.sqlite"), Path::to_path_buf);
        let mut index = mif_rh::FindingIndex::open(&index_path)?;
        mif_rh::build_search_index(&reports_dir, &all_topic_ids, &mut index)?;
    }

    let exit_ok = if args.strict {
        !report.strict_should_fail()
    } else {
        true
    };
    Ok(Outcome {
        message,
        exit_code: u8::from(!exit_ok),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        CalibrateArgs, ReviewArgs, SuggestTypeArgs, calibrate_cmd, expansion_candidates_cmd,
        resolve, review, suggest_type_cmd, topic_from_path,
    };

    fn write_fixture(dir: &std::path::Path) {
        fs::create_dir_all(dir.join(".claude")).unwrap();
        fs::create_dir_all(dir.join("packs")).unwrap();
        fs::create_dir_all(dir.join("reports/edu/findings")).unwrap();
        fs::write(
            dir.join("harness.config.json"),
            r#"{"topics":[{"id":"edu","ontologies":["edu-fixture"]}]}"#,
        )
        .unwrap();
        fs::write(
            dir.join(".claude/enabled-packs.json"),
            r#"{"ontologies":[{"id":"edu-fixture","version":"0.1.0","source":"packs/edu-fixture.yaml","core":false}]}"#,
        )
        .unwrap();
        fs::write(
            dir.join("packs/edu-fixture.yaml"),
            "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\nentity_types:\n  - name: title\n    schema:\n      required: [name]\n      properties: {name: {type: string}}\n",
        )
        .unwrap();
        fs::write(
            dir.join("reports/edu/findings/good.json"),
            r#"{"@id":"f-good","entity":{"name":"Algebra I","entity_type":"title"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("reports/edu/findings/invalid.json"),
            r#"{"@id":"f-invalid","entity":{"entity_type":"title"}}"#,
        )
        .unwrap();
    }

    #[test]
    fn resolve_a_valid_finding_exits_zero_and_writes_the_map() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        let map_path = dir.path().join("map.json");

        let outcome = resolve(
            &dir.path().join("reports/edu/findings/good.json"),
            Some("edu"),
            Some(&dir.path().join(".claude/enabled-packs.json")),
            Some(&dir.path().join("harness.config.json")),
            Some(&map_path),
            Some(dir.path()),
        )
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.message.contains("resolved"));
        assert!(map_path.exists());
    }

    #[test]
    fn resolve_an_invalid_finding_exits_nonzero() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());

        let outcome = resolve(
            &dir.path().join("reports/edu/findings/invalid.json"),
            Some("edu"),
            Some(&dir.path().join(".claude/enabled-packs.json")),
            Some(&dir.path().join("harness.config.json")),
            None,
            Some(dir.path()),
        )
        .unwrap();

        assert_eq!(outcome.exit_code, 1);
        assert!(outcome.message.contains("valid=false"));
    }

    #[test]
    fn suggest_type_prints_tier_annotated_json_and_exits_zero() {
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());

        let outcome = suggest_type_cmd(&SuggestTypeArgs {
            text: Some("A textbook titled Algebra I"),
            finding: None,
            topic: Some("edu"),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            config: Some(&dir.path().join("harness.config.json")),
            root: Some(dir.path()),
            limit: 10,
            calibration: Some(&dir.path().join("absent-calibration.json")),
            record: false,
            index: None,
        })
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        let suggestions: serde_json::Value = serde_json::from_str(&outcome.message).unwrap();
        let list = suggestions.as_array().unwrap();
        // The fixture pack's only entity type carries no description/aliases/
        // exemplars — no positive embedding signal, so it is skipped and the
        // hypothesis list is empty, which is still a successful outcome.
        assert!(list.is_empty());
    }

    #[test]
    fn expansion_candidates_on_a_fresh_index_emit_an_empty_cluster_list() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("index.sqlite");

        let outcome = expansion_candidates_cmd(
            Some(&index_path),
            Some(&dir.path().join("absent-calibration.json")),
            None,
        )
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        let payload: serde_json::Value = serde_json::from_str(&outcome.message).unwrap();
        assert!(payload["clusters"].as_array().unwrap().is_empty());
        assert_eq!(payload["misses_considered"], 0);
    }

    /// Enriches the fixture pack with a described entity type so suggest/
    /// calibrate paths have a positive embedding signal to work with.
    fn enrich_fixture_pack(dir: &std::path::Path) {
        fs::write(
            dir.join("packs/edu-fixture.yaml"),
            "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\nentity_types:\n  - name: title\n    description: A published educational title\n    aliases: [textbook]\n    schema:\n      required: [name]\n      properties: {name: {type: string}}\n",
        )
        .unwrap();
    }

    #[test]
    fn calibrate_derives_a_wellformed_artifact_from_stamped_findings() {
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        enrich_fixture_pack(dir.path());

        // Stamp the valid finding into the topic map first — stamped
        // findings are calibrate's labeled sample.
        resolve(
            &dir.path().join("reports/edu/findings/good.json"),
            Some("edu"),
            Some(&dir.path().join(".claude/enabled-packs.json")),
            Some(&dir.path().join("harness.config.json")),
            Some(&dir.path().join("reports/edu/ontology-map.json")),
            Some(dir.path()),
        )
        .unwrap();

        let out = dir.path().join("reports/_meta/confidence-calibration.json");
        let confusions_path = dir.path().join("reports/_meta/confusions.json");
        let outcome = calibrate_cmd(&CalibrateArgs {
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            root: Some(dir.path()),
            target_precision: 1.0,
            tier2_target: 0.5,
            sample: None,
            seed: 0,
            out: Some(&out),
            confusions: Some(&confusions_path),
        })
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        let cal = mif_ontology::CalibrationConfig::load_or_default(&out).unwrap();
        assert!(cal.calibrated);
        assert_eq!(cal.method.as_deref(), Some("stamped-quantile-v1"));
        assert_eq!(cal.sample_size, Some(1));
        assert!(cal.tier2_floor <= cal.tier1_floor);
        // The fixture pack carries no negative_examples, so the artifact
        // records that the demotion gate did not participate.
        assert!(!cal.negatives_active);

        // The confusion export was written alongside the artifact: the
        // fixture's one sample is correct, so the report is versioned,
        // counts the sample, and carries no pairs.
        assert!(outcome.message.contains("confusion pair(s)"));
        let report: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&confusions_path).unwrap()).unwrap();
        assert_eq!(report["version"], "confusions-v1");
        assert_eq!(report["sample_count"], 1);
        assert!(report["pairs"].as_array().unwrap().is_empty());
    }

    #[test]
    fn calibrate_records_negatives_participation_in_the_artifact() {
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        // Enrich the pack WITH a curated negative: the artifact must record
        // that the demotion gate participated in the swept scores.
        fs::write(
            dir.path().join("packs/edu-fixture.yaml"),
            "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\nentity_types:\n  - name: title\n    description: A published educational title\n    aliases: [textbook]\n    negative_examples:\n      - A lesson plan for teachers\n    schema:\n      required: [name]\n      properties: {name: {type: string}}\n",
        )
        .unwrap();

        resolve(
            &dir.path().join("reports/edu/findings/good.json"),
            Some("edu"),
            Some(&dir.path().join(".claude/enabled-packs.json")),
            Some(&dir.path().join("harness.config.json")),
            Some(&dir.path().join("reports/edu/ontology-map.json")),
            Some(dir.path()),
        )
        .unwrap();

        let out = dir.path().join("reports/_meta/confidence-calibration.json");
        let outcome = calibrate_cmd(&CalibrateArgs {
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            root: Some(dir.path()),
            target_precision: 1.0,
            tier2_target: 0.5,
            sample: None,
            seed: 0,
            out: Some(&out),
            confusions: None,
        })
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        let cal = mif_ontology::CalibrationConfig::load_or_default(&out).unwrap();
        assert!(cal.negatives_active);
    }

    #[test]
    fn an_unbound_negatives_pack_never_claims_participation() {
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        enrich_fixture_pack(dir.path());
        // A SECOND pack carrying negatives is enabled in the catalog but
        // not core and not bound to the scored topic: it never scores a
        // sample, so the artifact must record negatives_active: false.
        fs::write(
            dir.path().join("packs/unbound-fixture.yaml"),
            "ontology:\n  id: unbound-fixture\n  version: \"0.1.0\"\nentity_types:\n  - name: lesson\n    description: A lesson plan\n    negative_examples:\n      - A published textbook\n",
        )
        .unwrap();
        fs::write(
            dir.path().join(".claude/enabled-packs.json"),
            r#"{"ontologies":[{"id":"edu-fixture","version":"0.1.0","source":"packs/edu-fixture.yaml","core":false},{"id":"unbound-fixture","version":"0.1.0","source":"packs/unbound-fixture.yaml","core":false}]}"#,
        )
        .unwrap();

        resolve(
            &dir.path().join("reports/edu/findings/good.json"),
            Some("edu"),
            Some(&dir.path().join(".claude/enabled-packs.json")),
            Some(&dir.path().join("harness.config.json")),
            Some(&dir.path().join("reports/edu/ontology-map.json")),
            Some(dir.path()),
        )
        .unwrap();

        let out = dir.path().join("reports/_meta/confidence-calibration.json");
        let outcome = calibrate_cmd(&CalibrateArgs {
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            root: Some(dir.path()),
            target_precision: 1.0,
            tier2_target: 0.5,
            sample: None,
            seed: 0,
            out: Some(&out),
            confusions: None,
        })
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        let cal = mif_ontology::CalibrationConfig::load_or_default(&out).unwrap();
        assert!(!cal.negatives_active);
    }

    #[test]
    fn calibrate_writes_the_confusion_export_even_when_the_sweep_fails() {
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        enrich_fixture_pack(dir.path());
        // Deliberately no resolve/stamping: zero labeled samples makes the
        // sweep fail loud, and an uncalibratable corpus is exactly the one
        // whose confusions curation needs — the export must survive.

        let confusions_path = dir.path().join("reports/_meta/confusions.json");
        let result = calibrate_cmd(&CalibrateArgs {
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            root: Some(dir.path()),
            target_precision: 1.0,
            tier2_target: 0.5,
            sample: None,
            seed: 0,
            out: Some(&dir.path().join("reports/_meta/confidence-calibration.json")),
            confusions: Some(&confusions_path),
        });

        // Outcome has no Debug impl; mapping the success side away lets
        // unwrap_err assert the failure directly.
        let error = result.map(|_| ()).unwrap_err();
        assert!(error.to_string().contains("no stamped"));
        let report: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&confusions_path).unwrap()).unwrap();
        assert_eq!(report["version"], "confusions-v1");
        assert_eq!(report["sample_count"], 0);
        assert!(report["pairs"].as_array().unwrap().is_empty());
    }

    #[test]
    fn review_suggest_writes_a_topic_queue_for_followup_findings() {
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        enrich_fixture_pack(dir.path());
        // An untyped finding WITH content: suggestable (the fixture's
        // invalid finding has no indexable text and is skipped).
        fs::write(
            dir.path().join("reports/edu/findings/untyped.json"),
            r#"{"@id":"f-untyped","content":"A fascinating textbook about geometry"}"#,
        )
        .unwrap();

        let outcome = review(&ReviewArgs {
            topics: &[],
            strict: false,
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            followup: None,
            root: Some(dir.path()),
            relationship_script: None,
            build_index: false,
            index: None,
            suggest: true,
            calibration: None,
        })
        .unwrap();

        // The suggestions confirmation prints before the final summary line.
        let lines: Vec<&str> = outcome.message.lines().collect();
        let confirmation = lines
            .iter()
            .position(|l| l.starts_with("ontology-review: suggestions written"))
            .expect("suggestions confirmation line present");
        assert!(confirmation < lines.len() - 1);

        // The untyped finding (not durably stamped) got queued.
        let queue_path = dir.path().join("reports/_meta/suggestions/edu.json");
        let queue: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&queue_path).unwrap()).unwrap();
        let entries = queue["entries"].as_array().unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e["finding_id"] == "f-untyped" && e["status"] == "pending")
        );
    }

    #[test]
    fn suggest_type_ranks_a_described_type_from_a_finding_query() {
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: embedding model unavailable in this environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        // Enrich the fixture pack with a described, aliased entity type so a
        // real candidate exists.
        fs::write(
            dir.path().join("packs/edu-fixture.yaml"),
            "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\nentity_types:\n  - name: title\n    description: A published educational title\n    aliases: [textbook]\n    schema:\n      required: [name]\n      properties: {name: {type: string}}\n",
        )
        .unwrap();

        let outcome = suggest_type_cmd(&SuggestTypeArgs {
            text: None,
            finding: Some(&dir.path().join("reports/edu/findings/good.json")),
            topic: None, // derived from the finding's reports/<topic>/ path
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            config: Some(&dir.path().join("harness.config.json")),
            root: Some(dir.path()),
            limit: 10,
            calibration: Some(&dir.path().join("absent-calibration.json")),
            record: false,
            index: None,
        })
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        let suggestions: serde_json::Value = serde_json::from_str(&outcome.message).unwrap();
        let list = suggestions.as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["entity_type"], "title");
        assert!(list[0]["tier"].is_string());
        assert_eq!(list[0]["calibrated"], false);
    }

    #[test]
    fn review_strict_fails_closed_on_an_invalid_finding_but_succeeds_without_strict() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());

        let strict_outcome = review(&ReviewArgs {
            topics: &[],
            strict: true,
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            followup: None,
            root: Some(dir.path()),
            relationship_script: None,
            build_index: false,
            index: None,
            suggest: false,
            calibration: None,
        })
        .unwrap();
        assert_eq!(strict_outcome.exit_code, 1);
        assert!(strict_outcome.message.contains("1 invalid/unresolved"));

        let lenient_outcome = review(&ReviewArgs {
            topics: &[],
            strict: false,
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            followup: None,
            root: Some(dir.path()),
            relationship_script: None,
            build_index: false,
            index: None,
            suggest: false,
            calibration: None,
        })
        .unwrap();
        assert_eq!(lenient_outcome.exit_code, 0);
    }

    #[test]
    fn review_acquires_and_releases_the_lock_and_prints_the_topic_table() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());

        let outcome = review(&ReviewArgs {
            topics: &[],
            strict: false,
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            followup: None,
            root: Some(dir.path()),
            relationship_script: None,
            build_index: false,
            index: None,
            suggest: false,
            calibration: None,
        })
        .unwrap();

        assert!(outcome.message.contains("TOPIC"), "{}", outcome.message);
        assert!(outcome.message.contains("edu"), "{}", outcome.message);
        assert!(
            !dir.path().join("reports/_meta/.review.lock").exists(),
            "lock file must be released after review() returns"
        );
    }

    #[test]
    fn review_with_followup_prints_the_summary_line_last() {
        // `ontology-review.sh` always ends its stdout on the aggregate
        // summary line, even when `--followup` also prints a write
        // confirmation — callers that capture only the last line (like
        // `verify.sh`'s gate_m12) depend on this exact order.
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());
        let followup_path = dir.path().join("followup.json");

        let outcome = review(&ReviewArgs {
            topics: &[],
            strict: false,
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            followup: Some(&followup_path),
            root: Some(dir.path()),
            relationship_script: None,
            build_index: false,
            index: None,
            suggest: false,
            calibration: None,
        })
        .unwrap();

        let last_line = outcome.message.lines().next_back().unwrap();
        assert!(
            last_line.starts_with("1 topic(s);"),
            "last line should be the summary, got: {last_line}"
        );
    }

    #[test]
    fn review_with_build_index_populates_the_default_index_path() {
        // `--build-index` loads the real embedding model (network access on
        // a cold cache) — skip gracefully rather than fail the suite if it
        // is unavailable, matching `mif-embed`'s own test convention.
        if mif_embed::Embedder::load().is_err() {
            eprintln!("skipping: could not load embedding model");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path());

        review(&ReviewArgs {
            topics: &[],
            strict: false,
            reports_dir: Some(&dir.path().join("reports")),
            config: Some(&dir.path().join("harness.config.json")),
            catalog: Some(&dir.path().join(".claude/enabled-packs.json")),
            followup: None,
            root: Some(dir.path()),
            relationship_script: None,
            build_index: true,
            index: None,
            suggest: false,
            calibration: None,
        })
        .unwrap();

        assert!(
            dir.path()
                .join("reports/_meta/search-index.sqlite")
                .exists()
        );
    }

    #[test]
    fn topic_from_path_derives_the_topic_from_a_reports_relative_finding_path() {
        assert_eq!(
            topic_from_path(std::path::Path::new("reports/edu/findings/f.json")),
            Some("edu".to_string())
        );
        assert_eq!(
            topic_from_path(std::path::Path::new("/some/other/path.json")),
            None
        );
    }
}
