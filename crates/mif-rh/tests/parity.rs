//! M1 parity proof: `mif-rh` reproduces rht's real
//! `resolve-ontology.sh`/`ontology-review.sh` classification outcomes
//! against its own real fixture corpus
//! (`evals/fixtures/ontology/*`), read-only.
//!
//! rht is a sibling checkout in this workspace, not a `mif-rs` dependency —
//! its location is workspace-specific and won't exist in an isolated clone
//! of this repo (e.g. a fresh CI checkout of `mif-rs` alone), so by default
//! every test here skips cleanly (prints and returns) rather than failing
//! when it can't find rht, instead of hard-failing or silently reporting
//! false green. Override the path via `MIF_RH_PARITY_FIXTURES_ROOT` if rht
//! lives somewhere other than the default sibling location; set
//! `MIF_RH_PARITY_REQUIRED` to turn the skip into a hard failure.
//!
//! Two environment variables control fixture discovery and skip behavior:
//!
//! - `MIF_RH_PARITY_FIXTURES_ROOT` — explicit path to an rht checkout,
//!   overriding the default sibling-location probe.
//! - `MIF_RH_PARITY_REQUIRED` — when set (any value), a missing checkout is
//!   a hard test FAILURE instead of a skip. CI's dedicated parity job sets
//!   this so the gate is fail-closed: a broken rht checkout step can never
//!   silently turn the whole parity suite into a green no-op.
//!
//! Two environment variables control fixture discovery:
//!
//! - `MIF_RH_PARITY_FIXTURES_ROOT` — explicit path to an rht checkout,
//!   overriding the default sibling-location probe.
//! - `MIF_RH_PARITY_REQUIRED` — when set (any value), a missing checkout is
//!   a hard test FAILURE instead of a skip. CI's dedicated parity job sets
//!   this so the gate is fail-closed: a broken rht checkout step can never
//!   silently turn the whole parity suite into a green no-op.
//!
//! This whole file is test-only fixture-loading support (an integration
//! test target, not library code), so it exempts itself from
//! `unwrap_used`/`expect_used` the same way `#[cfg(test)]` unit test
//! modules already are elsewhere in this workspace (see `clippy.toml`'s
//! `allow-unwrap-in-tests`) — that setting only recognizes code directly
//! under a `#[test]` item, not the shared helpers this file factors out.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mif_rh::{Catalog, Finding, HarnessConfig, ResolveContext, resolve_finding};

fn rht_root() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MIF_RH_PARITY_FIXTURES_ROOT") {
        let path = PathBuf::from(path);
        return path.is_dir().then_some(path);
    }
    // From crates/mif-rh (this crate's manifest dir) to the workspace's
    // sibling `research-harness-template` checkout:
    // crates/mif-rh -> crates -> mif-rs-mif-rh (worktree root) -> worktrees
    // -> modeled-information-format -> repos/research-harness-template.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir
        .join("../../../../repos/research-harness-template")
        .canonicalize()
        .ok()?;
    candidate.is_dir().then_some(candidate)
}

struct Fixtures {
    root: PathBuf,
    catalog: Catalog,
    config: HarnessConfig,
    ontology_packs: HashMap<String, mif_rh::OntologyPack>,
}

impl Fixtures {
    fn load(root: &Path) -> Self {
        let fixtures_dir = root.join("evals/fixtures/ontology");
        let catalog = Catalog::load(&fixtures_dir.join("catalog.json")).unwrap();
        let config = HarnessConfig::load(&fixtures_dir.join("config.json")).unwrap();
        let ontology_packs = mif_rh::ontology_pack::load_packs_via_catalog(&catalog, root).unwrap();
        Self {
            root: root.to_path_buf(),
            catalog,
            config,
            ontology_packs,
        }
    }

    fn fixture_finding(&self, name: &str) -> Finding {
        Finding::load(&self.root.join("evals/fixtures/ontology").join(name)).unwrap()
    }

    const fn ctx<'a>(&'a self, topic: &'a str) -> ResolveContext<'a> {
        ResolveContext {
            topic,
            catalog: &self.catalog,
            config: &self.config,
            ontology_packs: &self.ontology_packs,
        }
    }
}

/// Loads the real rht fixture corpus, or `None` if no sibling checkout is
/// available in this workspace. When this is `None`, every test below
/// returns early (printing a skip notice) — or fails hard under
/// `MIF_RH_PARITY_REQUIRED` (see `skip_without_rht!`).
fn load_fixtures() -> Option<Fixtures> {
    let root = rht_root()?;
    Some(Fixtures::load(&root))
}

/// Expands to a [`Fixtures`] value; `return`s out of the calling test
/// (printing a skip notice) if no rht checkout is available — unless
/// `MIF_RH_PARITY_REQUIRED` is set, which makes a missing checkout a hard
/// failure (fail-closed for CI). Written as an
/// expression macro rather than one that introduces a `let` binding of its
/// own, since `macro_rules!` hygiene would make a binding created *inside*
/// the macro invisible to code after it — `return` (unlike an identifier)
/// crosses that boundary fine, so `let fixtures = skip_without_rht!();`
/// binds `fixtures` in the caller's own scope.
macro_rules! skip_without_rht {
    () => {
        match load_fixtures() {
            Some(fixtures) => fixtures,
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

/// eval `ontology-resolve-good` / `verify.sh` `gate_m12` "12e": `good.json` on
/// `edu` resolves cleanly.
#[test]
fn good_on_edu_resolves_and_validates() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("good.json");
    let record = resolve_finding(&finding, &fixtures.ctx("edu")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Resolved);
    assert!(record.valid);
    assert_eq!(
        record.resolved_ontology.as_deref(),
        Some("edu-fixture@0.1.0")
    );
}

/// eval `ontology-extra-field-ok`: an extra property is allowed additively.
#[test]
fn extra_field_is_allowed_additively() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("extra.json");
    let record = resolve_finding(&finding, &fixtures.ctx("edu")).unwrap();
    assert!(record.valid);
}

/// eval `ontology-untyped-ok`: an untyped finding with no discovery match.
#[test]
fn untyped_content_with_no_discovery_match_is_untyped() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("untyped.json");
    let record = resolve_finding(&finding, &fixtures.ctx("edu")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Untyped);
    assert!(record.valid);
}

/// eval `ontology-missing-required`: resolves but fails schema validation
/// (missing `isbn`/`grade_range`).
#[test]
fn missing_required_fields_resolves_but_is_invalid() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("missing.json");
    let record = resolve_finding(&finding, &fixtures.ctx("edu")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Resolved);
    assert!(!record.valid);
}

/// eval `ontology-undeclared-type`: `entity_type` not declared by any
/// allowed ontology.
#[test]
fn undeclared_type_is_unresolved() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("undecl.json");
    let record = resolve_finding(&finding, &fixtures.ctx("edu")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Unresolved);
    assert!(!record.valid);
}

/// eval `ontology-unbound-for-topic`: `good.json`'s `title` type is not
/// declared for the unbound `bare` topic.
#[test]
fn good_finding_type_is_unresolved_on_an_unbound_topic() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("good.json");
    let record = resolve_finding(&finding, &fixtures.ctx("bare")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Unresolved);
}

/// eval `ontology-generic-core`: `concept` resolves via the always-core
/// `mif-generic`, even on the unbound `bare` topic.
#[test]
fn generic_core_type_resolves_on_any_topic() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("generic.json");
    let record = resolve_finding(&finding, &fixtures.ctx("bare")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Resolved);
    assert!(record.valid);
    assert_eq!(
        record.resolved_ontology.as_deref(),
        Some("mif-generic@1.0.0")
    );
}

/// eval `ontology-ambiguous`: `technology` is declared by both the
/// always-core `mif-generic` and `eng`'s bound `collide-fixture`, with no
/// `ontology.id` to disambiguate.
#[test]
fn ambiguous_type_without_explicit_ontology_id_is_ambiguous() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("ambiguous.json");
    let record = resolve_finding(&finding, &fixtures.ctx("eng")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Ambiguous);
    assert!(!record.valid);
}

/// eval `ontology-disambiguated`: an explicit `ontology.id` resolves the
/// same collision cleanly.
#[test]
fn explicit_ontology_id_disambiguates_a_colliding_type() {
    let fixtures = skip_without_rht!();
    let finding = fixtures.fixture_finding("disambig.json");
    let record = resolve_finding(&finding, &fixtures.ctx("eng")).unwrap();
    assert_eq!(record.basis, mif_rh::Basis::Declared);
    assert_eq!(
        record.resolved_ontology.as_deref(),
        Some("collide-fixture@0.1.0")
    );
}

/// `verify.sh` `gate_m21`: `engineering-base` (reached transitively via
/// `software-engineering`'s `extends` chain) resolves on `eng`, but must
/// not leak into `edu`, which never extends it.
#[test]
fn transitive_extends_does_not_leak_across_unrelated_topics() {
    let fixtures = skip_without_rht!();
    // component is declared by engineering-base, reached transitively via
    // eng's bound software-engineering -> engineering-base chain.
    let component = serde_json::json!({
        "@id": "f-component-test",
        "entity": {"name": "auth-service", "entity_type": "component"}
    });
    let finding: Finding = serde_json::from_value(component).unwrap();

    let on_eng = resolve_finding(&finding, &fixtures.ctx("eng")).unwrap();
    assert_eq!(on_eng.basis, mif_rh::Basis::Resolved);
    assert_eq!(
        on_eng.resolved_ontology.as_deref(),
        Some("engineering-base@0.1.0")
    );

    let on_edu = resolve_finding(&finding, &fixtures.ctx("edu")).unwrap();
    assert_eq!(on_edu.basis, mif_rh::Basis::Unresolved);
}

/// `run-evals.sh` `ontology-review-coverage`: a single stamped finding
/// reviews clean under `--strict`.
#[test]
fn review_a_single_stamped_finding_passes_strict() {
    let fixtures = skip_without_rht!();
    let scratch = tempfile::tempdir().unwrap();
    let findings_dir = scratch.path().join("edu/findings");
    std::fs::create_dir_all(&findings_dir).unwrap();
    std::fs::copy(
        fixtures.root.join("evals/fixtures/ontology/good.json"),
        findings_dir.join("good.json"),
    )
    .unwrap();

    let opts = mif_rh::ReviewOptions {
        topics: Some(&["edu".to_string()]),
        reports_dir: scratch.path(),
        ontology_packs: &fixtures.ontology_packs,
        catalog: &fixtures.catalog,
        config: &fixtures.config,
        check_relationship_targets_script: None,
    };
    let (report, _backlog) = mif_rh::review(&opts).unwrap();
    assert!(!report.strict_should_fail());
    assert_eq!(
        report.summary_line(),
        "1 topic(s); 1 findings — 1 stamped, 0 discovery-only, 0 untyped, 0 invalid/unresolved"
    );
}

/// `run-evals.sh` `ontology-review-discovery-not-stamped`: a stamped
/// finding plus a discovery-only finding must NOT both read as "typed" —
/// this is the exact PR #251 regression the `--followup` backlog exists to
/// catch.
#[test]
fn review_distinguishes_stamped_from_discovery_only_in_the_summary_and_followup() {
    let fixtures = skip_without_rht!();
    let scratch = tempfile::tempdir().unwrap();
    let findings_dir = scratch.path().join("edu/findings");
    std::fs::create_dir_all(&findings_dir).unwrap();
    std::fs::copy(
        fixtures.root.join("evals/fixtures/ontology/good.json"),
        findings_dir.join("good.json"),
    )
    .unwrap();
    std::fs::copy(
        fixtures.root.join("evals/fixtures/ontology/discovery.json"),
        findings_dir.join("discovery.json"),
    )
    .unwrap();

    let opts = mif_rh::ReviewOptions {
        topics: Some(&["edu".to_string()]),
        reports_dir: scratch.path(),
        ontology_packs: &fixtures.ontology_packs,
        catalog: &fixtures.catalog,
        config: &fixtures.config,
        check_relationship_targets_script: None,
    };
    let (report, backlog) = mif_rh::review(&opts).unwrap();
    assert_eq!(
        report.summary_line(),
        "1 topic(s); 2 findings — 1 stamped, 1 discovery-only, 0 untyped, 0 invalid/unresolved"
    );
    assert_eq!(backlog.total_needs_followup, 1);
    let edu_followup = &backlog.topics["edu"];
    assert_eq!(edu_followup.len(), 1);
    assert_eq!(edu_followup[0].finding_id, "f-discovery-only");
    assert_eq!(edu_followup[0].basis, "discovery");
}

/// `verify.sh` `gate_m12` "12j": one stamped, one discovery-only, one
/// untyped, one invalid finding produces the exact documented summary
/// substring, and `--strict` fails closed on the one invalid record.
#[test]
fn review_gate_m12_mixed_corpus_matches_the_documented_summary_and_strict_exit() {
    let fixtures = skip_without_rht!();
    let scratch = tempfile::tempdir().unwrap();
    let findings_dir = scratch.path().join("edu/findings");
    std::fs::create_dir_all(&findings_dir).unwrap();
    for name in [
        "good.json",
        "discovery.json",
        "untyped.json",
        "missing.json",
    ] {
        std::fs::copy(
            fixtures.root.join("evals/fixtures/ontology").join(name),
            findings_dir.join(name),
        )
        .unwrap();
    }

    let opts = mif_rh::ReviewOptions {
        topics: Some(&["edu".to_string()]),
        reports_dir: scratch.path(),
        ontology_packs: &fixtures.ontology_packs,
        catalog: &fixtures.catalog,
        config: &fixtures.config,
        check_relationship_targets_script: None,
    };
    let (report, _backlog) = mif_rh::review(&opts).unwrap();
    assert!(
        report
            .summary_line()
            .contains("1 stamped, 1 discovery-only, 1 untyped, 1 invalid")
    );
    assert!(report.strict_should_fail());
}
