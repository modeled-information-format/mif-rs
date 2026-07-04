//! Shared fixture helpers for `mif-rh-cli`'s binary-level integration
//! tests.
//!
//! This is a non-root module of each test target that declares `mod
//! common;`, so it carries no crate-level `#![allow(...)]` header of its
//! own — it inherits the including target's (see `bin_parity.rs`, which
//! exempts itself from `unwrap_used`/`expect_used` the same way
//! `mif-rh`'s `parity.rs` does).

use std::fs;
use std::path::{Path, PathBuf};

/// Locates the sibling research-harness-template (rht) checkout, exactly
/// like `mif-rh`'s `parity.rs`: `MIF_RH_PARITY_FIXTURES_ROOT` wins if set;
/// otherwise probe the workspace-sibling default. `crates/mif-rh-cli` sits
/// at the same depth as `crates/mif-rh`, so the same relative hop
/// (`../../../../repos/research-harness-template`) applies.
pub fn rht_root() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MIF_RH_PARITY_FIXTURES_ROOT") {
        let path = PathBuf::from(path);
        return path.is_dir().then_some(path);
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir
        .join("../../../../repos/research-harness-template")
        .canonicalize()
        .ok()?;
    candidate.is_dir().then_some(candidate)
}

/// Writes a fully self-contained fixture corpus (no rht checkout needed)
/// into `dir`, mirroring the layout `mif-rh-cli`'s defaults expect when run
/// with `dir` as its working directory:
///
/// - `harness.config.json` — one `edu` topic bound to `edu-fixture`
/// - `.claude/enabled-packs.json` — the ontology catalog
/// - `packs/edu-fixture.yaml` — one `title` entity type requiring `name`
/// - `reports/edu/findings/good.json` — valid (`f-good`)
/// - `reports/edu/findings/invalid.json` — missing the required `name`
///   entity field (`f-invalid`)
/// - `reports/edu/findings/untyped.json` — no entity, no discovery match
///   (`f-untyped`)
///
/// Reviewing it yields 3 findings: 1 stamped, 0 discovery-only, 1 untyped,
/// 1 invalid/unresolved.
pub fn write_minimal_corpus(dir: &Path) {
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
    fs::write(
        dir.join("reports/edu/findings/untyped.json"),
        r#"{"@id":"f-untyped","content":"x"}"#,
    )
    .unwrap();
}
