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
        #[arg(long)]
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
        }),
        Command::ExpansionCandidates {
            index,
            calibration,
            out,
        } => expansion_candidates_cmd(index.as_deref(), calibration.as_deref(), out.as_deref()),
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
    let (query, topic) = if let Some(finding_path) = args.finding {
        let finding = mif_rh::Finding::load(finding_path)?;
        let topic = args
            .topic
            .map(str::to_string)
            .or_else(|| topic_from_path(finding_path))
            .unwrap_or_default();
        (mif_rh::index_text(&finding), topic)
    } else {
        (
            args.text.unwrap_or_default().to_string(),
            args.topic.unwrap_or_default().to_string(),
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
    let suggestions = mif_rh::suggest_type(&query, &ctx, &embedder, &cal, args.limit)?;

    if args.record && is_expansion_miss(&suggestions) {
        // clap's `requires = "finding"` guarantees a finding path here.
        if let Some(finding_path) = args.finding {
            let finding = mif_rh::Finding::load(finding_path)?;
            let index_path = effective_path(args.index, DEFAULT_INDEX);
            let index = mif_rh::FindingIndex::open(&index_path)?;
            index.record_miss(&mif_rh::Miss {
                finding_id: finding.id,
                topic: topic.clone(),
                content: query.clone(),
                vector: embedder.embed(&query)?,
                run_id: run_id(),
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
    let cal = mif_rh::sweep(&samples, &opts, &out)?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|source| mif_rh::MifRhError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    mif_rh::write_json_atomic(&out, &cal)?;

    let message = format!(
        "calibrate: {} sample(s) -> tier1_floor={:.2} tier1_margin={:.2} tier2_floor={:.2} \
         (method {}, written to {})",
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
    let idx = mif_rh::FindingIndex::open(&index_path)?;
    let misses = idx.misses()?;
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
            let suggestions = mif_rh::suggest_type(&query, &ctx, &embedder, &cal, 5)?;
            if is_expansion_miss(&suggestions) {
                index.record_miss(&mif_rh::Miss {
                    finding_id: finding.id.clone(),
                    topic: topic_id.clone(),
                    vector: embedder.embed(&query)?,
                    content: query,
                    run_id: run.clone(),
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
        })
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        let cal = mif_ontology::CalibrationConfig::load_or_default(&out).unwrap();
        assert!(cal.calibrated);
        assert_eq!(cal.method.as_deref(), Some("stamped-quantile-v1"));
        assert_eq!(cal.sample_size, Some(1));
        assert!(cal.tier2_floor <= cal.tier1_floor);
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

        // The invalid finding (not durably stamped) got queued.
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
