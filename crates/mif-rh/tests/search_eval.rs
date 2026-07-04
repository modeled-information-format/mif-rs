//! M3 search eval: cross-topic semantic recall over rht's curated
//! known-similar finding pairs (`evals/fixtures/search/known-similar-pairs.json`).
//! For every pair, each member's counterpart must rank in the top-K results
//! of [`mif_rh::FindingIndex::find_similar`] when every other member of the
//! fixture serves as a distractor.
//!
//! Fixture discovery:
//!
//! 1. `MIF_RH_SEARCH_EVAL_FIXTURE` — path to the known-similar-pairs JSON
//!    file itself (lets local runs point at a draft fixture before the rht
//!    branch that adds it lands). Setting it TERMINATES discovery: a
//!    missing file at that path is a hard failure, never a fallback to
//!    step 2.
//! 2. Otherwise `MIF_RH_PARITY_FIXTURES_ROOT` (or the default sibling
//!    `research-harness-template` checkout, same as `parity.rs`) joined
//!    with `evals/fixtures/search/known-similar-pairs.json`.
//!
//! Without a fixture, or when the embedding model can't load, these tests
//! skip (print and return) — unless `MIF_RH_SEARCH_EVAL_REQUIRED` is set
//! (any value, matching the parity suite's own required-switch
//! convention), which makes either condition a hard failure. This
//! fail-closed switch is deliberately its own variable, separate from the
//! parity suite's: CI pins an rht SHA that won't contain this fixture
//! until the companion rht PR merges and the pin is bumped, so the search
//! eval must be requirable independently. The pin-bump follow-up sets
//! `MIF_RH_SEARCH_EVAL_REQUIRED` in the CI parity job — until then this
//! eval runs locally only.
//!
//! This whole file is test-only support (an integration test target, not
//! library code), so it exempts itself from `unwrap_used`/`expect_used`/
//! `print_stderr` the same way `#[cfg(test)]` unit test modules already are
//! elsewhere in this workspace (see `clippy.toml`'s `allow-unwrap-in-tests`/
//! `allow-print-in-tests`) — those settings only recognize code directly
//! under a `#[test]` item, not the shared helpers this file factors out
//! (the skip notices print from `load_fixture`, a plain helper fn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::Deserialize;

use mif_rh::{FindingIndex, IndexedFinding};

#[derive(Debug, Deserialize)]
struct Fixture {
    top_k_default: usize,
    pairs: Vec<Pair>,
}

#[derive(Debug, Deserialize)]
struct Pair {
    id: String,
    cross_topic: bool,
    /// Optional per-pair override of the fixture-wide `top_k_default`.
    #[serde(default)]
    top_k: Option<usize>,
    a: Member,
    b: Member,
}

#[derive(Debug, Deserialize)]
struct Member {
    finding_id: String,
    topic: String,
    text: String,
}

fn required() -> bool {
    std::env::var_os("MIF_RH_SEARCH_EVAL_REQUIRED").is_some()
}

fn rht_root() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MIF_RH_PARITY_FIXTURES_ROOT") {
        let path = PathBuf::from(path);
        return path.is_dir().then_some(path);
    }
    // From crates/mif-rh (this crate's manifest dir) to the workspace's
    // sibling `research-harness-template` checkout — same convention as
    // `parity.rs`.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir
        .join("../../../../repos/research-harness-template")
        .canonicalize()
        .ok()?;
    candidate.is_dir().then_some(candidate)
}

fn fixture_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MIF_RH_SEARCH_EVAL_FIXTURE") {
        let path = PathBuf::from(path);
        // An explicit override naming a missing file is a configuration
        // error, not fixture absence — skipping here would be fail-open,
        // and setting the variable terminates discovery (no fallback to
        // the rht probe below).
        assert!(
            path.is_file(),
            "MIF_RH_SEARCH_EVAL_FIXTURE is set but {} is not a file",
            path.display()
        );
        return Some(path);
    }
    let candidate = rht_root()?.join("evals/fixtures/search/known-similar-pairs.json");
    candidate.is_file().then_some(candidate)
}

/// Loads the known-similar-pairs fixture, or `None` (after printing a skip
/// notice) if no fixture is available. A *present but unreadable/invalid*
/// fixture is always a hard failure — only absence is skippable, and only
/// when `MIF_RH_SEARCH_EVAL_REQUIRED` is unset.
fn load_fixture() -> Option<Fixture> {
    let Some(path) = fixture_path() else {
        assert!(
            !required(),
            "MIF_RH_SEARCH_EVAL_REQUIRED is set but no known-similar-pairs fixture was found \
             (set MIF_RH_SEARCH_EVAL_FIXTURE to the fixture file, or \
             MIF_RH_PARITY_FIXTURES_ROOT to an rht checkout containing \
             evals/fixtures/search/known-similar-pairs.json)"
        );
        eprintln!(
            "skipping: no known-similar-pairs fixture found (set MIF_RH_SEARCH_EVAL_FIXTURE \
             or MIF_RH_PARITY_FIXTURES_ROOT to override)"
        );
        return None;
    };
    let raw = std::fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))
        .expect("fixture file should be readable");
    let fixture: Fixture = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
        .expect("fixture file should be valid known-similar-pairs JSON");
    Some(fixture)
}

/// The fixture itself is structurally sound: enough pairs to be a
/// meaningful eval, at least one cross-topic pair (the recall property M3
/// actually cares about), and no finding id reused across members (each
/// member must be a distinct index entry, so every non-counterpart is a
/// genuine distractor).
#[test]
fn fixture_is_well_formed() {
    let Some(fixture) = load_fixture() else {
        return;
    };
    assert!(
        fixture.pairs.len() >= 10,
        "expected at least 10 pairs, got {}",
        fixture.pairs.len()
    );
    assert!(
        fixture.pairs.iter().any(|pair| pair.cross_topic),
        "expected at least one cross_topic pair"
    );
    let mut seen = HashSet::new();
    for pair in &fixture.pairs {
        for member in [&pair.a, &pair.b] {
            assert!(
                seen.insert(member.finding_id.as_str()),
                "duplicate finding_id across members: {}",
                member.finding_id
            );
        }
    }
}

/// Every known-similar pair's counterpart ranks in the top-K similarity
/// results, in both directions, with every other fixture member indexed as
/// a distractor. Failures are collected (not asserted one at a time) so a
/// partial regression names every missed pair + direction.
#[test]
fn known_similar_pairs_rank_each_counterpart_in_top_k() {
    let Some(fixture) = load_fixture() else {
        return;
    };
    let embedder = match mif_embed::Embedder::load() {
        Ok(embedder) => embedder,
        Err(err) => {
            assert!(
                !required(),
                "MIF_RH_SEARCH_EVAL_REQUIRED is set but the embedding model failed to load: {err}"
            );
            eprintln!("skipping: embedding model unavailable ({err})");
            return;
        },
    };

    // This test enforces the invariants it relies on rather than trusting
    // fixture_is_well_formed alone: a filtered run of just this test must
    // still validate them, and a shared finding_id must fail loudly here
    // instead of silently shrinking the distractor set.
    assert!(
        !fixture.pairs.is_empty(),
        "fixture has no pairs — the ranking assertions below would pass vacuously"
    );
    let mut embeddings: HashMap<String, Vec<f32>> = HashMap::new();
    let mut indexed = Vec::new();
    for pair in &fixture.pairs {
        for member in [&pair.a, &pair.b] {
            let vector = embedder
                .embed(&member.text)
                .map_err(|err| format!("failed to embed {}: {err}", member.finding_id))
                .expect("member text should embed");
            let fresh = embeddings
                .insert(member.finding_id.clone(), vector.clone())
                .is_none();
            assert!(
                fresh,
                "finding_id {} appears in more than one pair member — every member must be a \
                 distinct index entry",
                member.finding_id
            );
            indexed.push(IndexedFinding {
                finding_id: member.finding_id.clone(),
                topic: member.topic.clone(),
                content: member.text.clone(),
                vector,
            });
        }
    }

    let scratch = tempfile::tempdir().unwrap();
    let mut index = FindingIndex::open(&scratch.path().join("search-eval.sqlite")).unwrap();
    index.rebuild(&indexed).unwrap();

    let mut failures = Vec::new();
    for pair in &fixture.pairs {
        let top_k = pair.top_k.unwrap_or(fixture.top_k_default);
        for (query, counterpart) in [(&pair.a, &pair.b), (&pair.b, &pair.a)] {
            let matches = index
                .find_similar(
                    &embeddings[&query.finding_id],
                    top_k,
                    Some(&query.finding_id),
                )
                .unwrap();
            if !matches
                .iter()
                .any(|m| m.finding_id == counterpart.finding_id)
            {
                failures.push(format!(
                    "{}: {} -> {} not in top-{top_k}",
                    pair.id, query.finding_id, counterpart.finding_id
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "known-similar counterpart(s) missed the top-k:\n{}",
        failures.join("\n")
    );
}
