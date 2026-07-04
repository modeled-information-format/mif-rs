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
        }),
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

    use super::{ReviewArgs, resolve, review, topic_from_path};

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
