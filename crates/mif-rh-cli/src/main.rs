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
        } => review(
            topic,
            *strict,
            reports_dir.as_deref(),
            config.as_deref(),
            catalog.as_deref(),
            followup.as_deref(),
            root.as_deref(),
        ),
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

    let json = serde_json::to_string_pretty(&records).unwrap_or_else(|_| "[]".to_string());
    let tmp_path = map_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json).map_err(|source| mif_rh::MifRhError::Io {
        path: tmp_path.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp_path, map_path).map_err(|source| mif_rh::MifRhError::Io {
        path: map_path.display().to_string(),
        source,
    })
}

#[allow(clippy::too_many_arguments)]
fn review(
    topics: &[String],
    strict: bool,
    reports_dir: Option<&Path>,
    config: Option<&Path>,
    catalog: Option<&Path>,
    followup: Option<&Path>,
    root: Option<&Path>,
) -> Result<Outcome, CliError> {
    let reports_dir = effective_path(reports_dir, DEFAULT_REPORTS_DIR);
    let config_path = effective_path(config, DEFAULT_CONFIG);
    let catalog_path = effective_path(catalog, DEFAULT_CATALOG);
    let root = effective_path(root, ".");

    let catalog = mif_rh::Catalog::load(&catalog_path)?;
    let config = mif_rh::HarnessConfig::load(&config_path)?;
    let ontology_packs = mif_rh::ontology_pack::load_packs_via_catalog(&catalog, &root)?;

    let topic_ids: Option<Vec<String>> = (!topics.is_empty()).then(|| topics.to_vec());
    let opts = mif_rh::ReviewOptions {
        topics: topic_ids.as_deref(),
        reports_dir: &reports_dir,
        ontology_packs: &ontology_packs,
        catalog: &catalog,
        config: &config,
        check_relationship_targets_script: None,
    };

    let (report, backlog) = mif_rh::review(&opts)?;

    if let Some(followup_path) = followup {
        mif_rh::write_followup(followup_path, &backlog)?;
    }

    let exit_ok = if strict {
        !report.strict_should_fail()
    } else {
        true
    };
    Ok(Outcome {
        message: report.summary_line(),
        exit_code: u8::from(!exit_ok),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{resolve, review, topic_from_path};

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

        let strict_outcome = review(
            &[],
            true,
            Some(&dir.path().join("reports")),
            Some(&dir.path().join("harness.config.json")),
            Some(&dir.path().join(".claude/enabled-packs.json")),
            None,
            Some(dir.path()),
        )
        .unwrap();
        assert_eq!(strict_outcome.exit_code, 1);
        assert!(strict_outcome.message.contains("1 invalid/unresolved"));

        let lenient_outcome = review(
            &[],
            false,
            Some(&dir.path().join("reports")),
            Some(&dir.path().join("harness.config.json")),
            Some(&dir.path().join(".claude/enabled-packs.json")),
            None,
            Some(dir.path()),
        )
        .unwrap();
        assert_eq!(lenient_outcome.exit_code, 0);
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
