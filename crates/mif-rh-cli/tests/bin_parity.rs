//! Binary-level parity tests: invoke the real `mif-rh-cli` executable
//! (via `CARGO_BIN_EXE_mif-rh-cli`) end-to-end and assert its exact
//! stdout/stderr/exit-code contracts.
//!
//! These are golden-expectation tests, not live diffs against rht's bash
//! scripts. Diffing against `resolve-ontology.sh`/`ontology-review.sh` at
//! test time would require bash plus yq/jq/ajv on every runner — including
//! Windows, where the bash side doesn't run at all — and rht's own CI
//! already proves the bash side against the same fixtures. What the binary
//! must hold stable is the byte-format contracts the code documents:
//! `format_topic_table`'s `printf '%-28s %-22s %6s %8s %10s %8s %9s'`
//! layout, `ReviewReport::summary_line`'s exact wording, and the
//! followup-confirmation-before-final-summary stdout ordering that
//! `verify.sh`'s `gate_m12` depends on when it captures only the last
//! stdout line. Those contracts are asserted here as literal strings, so
//! any drift in the binary's output shows up as a test failure naming the
//! exact bytes that changed.
//!
//! The one rht-fixture-gated test mirrors `mif-rh`'s `parity.rs`
//! conventions: it skips (printing a notice) when no rht checkout is
//! found, unless `MIF_RH_PARITY_REQUIRED` is set, in which case a missing
//! checkout is a hard failure — CI's parity job sets it so the gate is
//! fail-closed.
//!
//! Like `parity.rs`, this whole file is test-only support, so it exempts
//! itself from `unwrap_used`/`expect_used` (`clippy.toml`'s
//! `allow-unwrap-in-tests` only recognizes code directly under a `#[test]`
//! item, not an integration-test target's shared helpers).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::process::{Command, Output};

/// The exact header + `edu` topic row `format_topic_table` prints for the
/// minimal corpus (3 findings: 1 stamped, 1 untyped, 1 invalid), matching
/// `ontology-review.sh`'s `printf '%-28s %-22s %6s %8s %10s %8s %9s'`.
const TABLE_HEADER: &str = "TOPIC                        BOUND                    FIND  STAMPED  DISCOVERY  UNTYPED   INVALID";
const MINIMAL_EDU_ROW: &str = "edu                          edu-fixture                 3        1          0        1         1";

fn bin(cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mif-rh-cli"));
    cmd.current_dir(cwd);
    cmd
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// Expands to the rht checkout root; `return`s out of the calling test
/// (printing a skip notice) if none is available — unless
/// `MIF_RH_PARITY_REQUIRED` is set, which turns the skip into a hard
/// failure (see `mif-rh`'s `parity.rs` for the same fail-closed contract).
macro_rules! skip_without_rht {
    () => {
        match common::rht_root() {
            Some(root) => root,
            None => {
                assert!(
                    std::env::var_os("MIF_RH_PARITY_REQUIRED").is_none(),
                    "MIF_RH_PARITY_REQUIRED is set but no research-harness-template \
                     checkout was found (set MIF_RH_PARITY_FIXTURES_ROOT to a valid \
                     rht checkout) — refusing to skip parity fail-open"
                );
                eprintln!(
                    "skipping: no research-harness-template checkout found (set \
                     MIF_RH_PARITY_FIXTURES_ROOT to override)"
                );
                return;
            },
        }
    };
}

/// `resolve-ontology.sh` parity: a valid finding prints the exact record
/// line and exits 0.
#[test]
fn resolve_a_valid_finding_prints_the_record_line_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    common::write_minimal_corpus(dir.path());

    let output = bin(dir.path())
        .args([
            "resolve",
            "reports/edu/findings/good.json",
            "--topic",
            "edu",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output).trim_end(),
        "f-good: resolved -> edu-fixture@0.1.0 (valid=true)"
    );
}

/// `resolve-ontology.sh` parity: an invalid finding (missing required
/// entity field) still resolves but reports `valid=false` and exits 1.
#[test]
fn resolve_an_invalid_finding_reports_valid_false_and_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    common::write_minimal_corpus(dir.path());

    let output = bin(dir.path())
        .args([
            "resolve",
            "reports/edu/findings/invalid.json",
            "--topic",
            "edu",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_of(&output).trim_end(),
        "f-invalid: resolved -> edu-fixture@0.1.0 (valid=false)"
    );
}

/// `ontology-review.sh` parity: the exact topic table layout, the `---`
/// separator, and the aggregate summary as the final stdout line; a corpus
/// with an invalid finding still exits 0 without `--strict`.
#[test]
fn review_prints_the_exact_topic_table_and_ends_on_the_summary_line() {
    let dir = tempfile::tempdir().unwrap();
    common::write_minimal_corpus(dir.path());

    let output = bin(dir.path()).arg("review").output().unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let stdout = stdout_of(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], TABLE_HEADER);
    assert_eq!(lines[1], MINIMAL_EDU_ROW);
    assert_eq!(lines[2], "---");
    assert_eq!(
        lines.last().copied().unwrap(),
        "1 topic(s); 3 findings — 1 stamped, 0 discovery-only, 1 untyped, 1 invalid/unresolved"
    );
}

/// `--strict` fails closed (exit 1) on a corpus with an invalid finding,
/// matching `ontology-review.sh`'s exit-code contract.
#[test]
fn review_strict_exits_one_on_an_invalid_finding() {
    let dir = tempfile::tempdir().unwrap();
    common::write_minimal_corpus(dir.path());

    let output = bin(dir.path())
        .args(["review", "--strict"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
}

/// `--followup` writes a parseable backlog JSON, and its confirmation line
/// prints BEFORE the final summary line — `verify.sh`'s `gate_m12` captures
/// only the last stdout line and depends on that order.
#[test]
fn review_followup_writes_the_backlog_and_prints_its_confirmation_before_the_summary() {
    let dir = tempfile::tempdir().unwrap();
    common::write_minimal_corpus(dir.path());
    let followup_path = dir.path().join("followup.json");

    let output = bin(dir.path())
        .args(["review", "--followup"])
        .arg(&followup_path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));

    let backlog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&followup_path).unwrap()).unwrap();
    assert_eq!(backlog["total_needs_followup"], 2);

    let stdout = stdout_of(&output);
    let confirmation = stdout
        .lines()
        .position(|l| l.starts_with("ontology-review: followup backlog written to"))
        .expect("followup confirmation line must be printed");
    let summary = stdout
        .lines()
        .position(|l| l.starts_with("1 topic(s);"))
        .expect("summary line must be printed");
    assert!(
        confirmation < summary,
        "followup confirmation must print before the final summary:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .next_back()
            .unwrap()
            .starts_with("1 topic(s);"),
        "the summary must be the final stdout line:\n{stdout}"
    );
}

/// clap's contract for an unrecognized subcommand: exit 2, usage on stderr.
#[test]
fn unknown_subcommand_exits_two_with_usage_on_stderr() {
    let dir = tempfile::tempdir().unwrap();

    let output = bin(dir.path()).arg("bogus").output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr_of(&output).contains("Usage:"),
        "stderr must carry usage: {}",
        stderr_of(&output)
    );
}

/// With `--format json`, a failed review renders stderr as an RFC 9457
/// problem document (machine-parseable `type`/`status` members), not
/// pretty text.
#[test]
fn review_in_an_empty_dir_with_format_json_emits_a_problem_document_on_stderr() {
    let dir = tempfile::tempdir().unwrap();

    let output = bin(dir.path())
        .args(["review", "--format", "json"])
        .output()
        .unwrap();

    assert_ne!(output.status.code(), Some(0));
    let stderr = stderr_of(&output);
    let problem: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr must parse as JSON");
    assert!(
        problem["type"].is_string(),
        "problem must have a type URI: {stderr}"
    );
    assert!(
        problem["status"].is_number(),
        "problem must have a status: {stderr}"
    );
}

/// `verify.sh` `gate_m12` "12j" against the real rht fixture corpus,
/// through the binary: one stamped, one discovery-only, one untyped, one
/// invalid finding produce the exact documented stdout tail, and
/// `--strict` exits 1. The corpus is staged into a scratch dir (catalog,
/// config, every catalog-referenced pack, four findings) so no rht script
/// is auto-discovered at `<root>/scripts/`.
#[test]
fn review_gate_m12_mixed_rht_corpus_matches_the_documented_output_through_the_binary() {
    let root = skip_without_rht!();
    let fixtures = root.join("evals/fixtures/ontology");
    let dir = tempfile::tempdir().unwrap();
    let scratch = dir.path();

    for name in ["catalog.json", "config.json"] {
        std::fs::copy(fixtures.join(name), scratch.join(name)).unwrap();
    }
    let catalog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures.join("catalog.json")).unwrap())
            .unwrap();
    for entry in catalog["ontologies"].as_array().unwrap() {
        let source = entry["source"].as_str().unwrap();
        let dest = scratch.join(source);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(root.join(source), dest).unwrap();
    }
    let findings_dir = scratch.join("reports/edu/findings");
    std::fs::create_dir_all(&findings_dir).unwrap();
    for name in [
        "good.json",
        "discovery.json",
        "untyped.json",
        "missing.json",
    ] {
        std::fs::copy(fixtures.join(name), findings_dir.join(name)).unwrap();
    }

    let review_args = [
        "review",
        "--topic",
        "edu",
        "--reports-dir",
        "reports",
        "--config",
        "config.json",
        "--catalog",
        "catalog.json",
        "--root",
        ".",
    ];
    let output = bin(scratch).args(review_args).output().unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let stdout = stdout_of(&output);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], TABLE_HEADER);
    assert_eq!(
        lines[1],
        "edu                          edu-fixture                 4        1          1        1         1"
    );
    assert_eq!(lines[2], "---");
    assert_eq!(
        lines.last().copied().unwrap(),
        "1 topic(s); 4 findings — 1 stamped, 1 discovery-only, 1 untyped, 1 invalid/unresolved"
    );

    let strict = bin(scratch)
        .args(review_args)
        .arg("--strict")
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(1));
}

/// R-4: on Windows, spawning a `.sh` relationship script cannot work
/// (`CreateProcess` does not honor shebangs), and the failure must be loud
/// — a nonzero exit with the spawn error on stderr — never a silently
/// skipped check with a normal-looking summary.
#[cfg(windows)]
#[test]
fn review_with_a_configured_relationship_script_fails_loud_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    common::write_minimal_corpus(dir.path());
    let script = dir.path().join("check.sh");
    std::fs::write(&script, "#!/usr/bin/env bash\nexit 0\n").unwrap();

    let output = bin(dir.path())
        .args(["review", "--relationship-script"])
        .arg(&script)
        .output()
        .unwrap();

    assert_ne!(
        output.status.code(),
        Some(0),
        "a spawn failure must not exit 0"
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("check.sh"),
        "stderr must name the script that failed to spawn: {stderr}"
    );
    let stdout = stdout_of(&output);
    assert!(
        !stdout.contains("topic(s);"),
        "a failed review must not print the normal summary line: {stdout}"
    );
}

/// The configured relationship script runs exactly once, corpus-wide —
/// not once per topic or per finding.
#[cfg(unix)]
#[test]
fn review_runs_a_configured_relationship_script_exactly_once() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    common::write_minimal_corpus(dir.path());
    let marker = dir.path().join("marker.txt");
    let script = dir.path().join("check.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho ran >> \"{}\"\nexit 0\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let output = bin(dir.path())
        .args(["review", "--relationship-script"])
        .arg(&script)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", stderr_of(&output));
    let runs = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(
        runs.lines().count(),
        1,
        "the relationship script must run exactly once, got: {runs:?}"
    );
}
