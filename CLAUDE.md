# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`mif-rs` is a Cargo **workspace** implementing the [MIF (Modeled Information
Format)](https://mif-spec.dev) specification in Rust (edition 2024, MSRV
1.92). Nine members, in dependency order:

| Crate | Kind | Purpose |
|---|---|---|
| `mif-core` | library | Shared types: `OntologyReference`, `EntityReference`, `EntityData`, `ConceptType` |
| `mif-problem` | library | Shared RFC 9457 Problem Details envelope (`ProblemDetails`, `ToProblem` trait, `OutputFormat`) every other crate's error enum maps into |
| `mif-schema` | library | JSON Schema validation of MIF documents/citations/ontology definitions |
| `mif-ontology` | library | Three-tier ontology `extends` chain resolution |
| `mif-frontmatter` | library | Markdown-frontmatter <-> JSON-LD projection and lossless round-trip proof, ported from the `MIF` repo's `mif_convert.py`, generalized beyond it (see "Why Generic Frontmatter Pass-Through" below) |
| `mif-embed` | library | Local (offline-after-first-fetch) sentence embeddings via `candle`, `sentence-transformers/all-MiniLM-L6-v2` |
| `mif-store` | library | `SQLite`-backed vector store for document embeddings (`rusqlite`, bundled), with brute-force cosine-similarity ranking (`top_k_similar`) |
| `mif-cli` | binary | CLI: `mif-cli validate <file>`, `mif-cli ontology resolve <id> --ontologies-dir <dir>`, `mif-cli ingest <file> [--db-path <path>]`, `mif-cli search <query>`, `mif-cli find-similar <id>`, `mif-cli corpus-stats` |
| `mif-mcp` | binary | MCP server exposing `validate_mif_document`, `resolve_ontology_reference`, `ingest_mif_document`, `search_documents`, `find_similar_documents`, and `corpus_stats` as tools |

Source for each crate lives at `crates/<name>/src/`. This is a **virtual
workspace** (the root `Cargo.toml` has no `[package]` section) — shared
lints live in `[workspace.lints]`, shared release profiles in the root
`[profile.*]` tables, and every member opts in via `[lints] workspace = true`.

---

## Documentation Standard: Diátaxis

All documentation in this project follows the [Diátaxis framework](https://diataxis.fr/). When adding or updating documentation — including this file, `docs/`, doc comments, and README files — classify content into one of four modes:

| Mode | Purpose | Prompt | Example |
|---|---|---|---|
| **How-to** | Task-oriented steps | "How do I…?" | Adding a new error variant, running tests |
| **Reference** | Precise, factual lookup | "What is…?" | Lint tables, cargo profiles, API signatures |
| **Explanation** | Design rationale | "Why does…?" | Why `thiserror`, why `panic = "abort"` |
| **Tutorial** | Learning-oriented walkthrough | "Teach me…" | (Not used in CLAUDE.md; use `docs/` for tutorials) |

**Rules for contributors (human and AI):**

- Before writing documentation, decide which Diátaxis mode it belongs to. Do not mix modes in a single section.
- **How-to** sections use numbered steps and end with a verification command.
- **Reference** sections use tables or structured lists. No rationale — just facts.
- **Explanation** sections use "Why X" headings and focus on trade-offs and decisions.
- New `docs/` files must declare their Diátaxis mode in a frontmatter comment or heading.
- When extending this CLAUDE.md, place new content under the correct Diátaxis heading below.

---

<!-- Diátaxis: How-to Guides — task-oriented, practical steps -->

## How-to Guides

### Build and Run

[`just`](https://github.com/casey/just) is the local task runner. Run `just` to list all recipes.

```bash
just                  # List all recipes
just check            # Full CI check (fmt + clippy + test + doc + deny), workspace-wide
just build            # Debug build (workspace)
just build-release    # Release build (workspace)
```

<details>
<summary>Raw cargo equivalents</summary>

```bash
cargo build --workspace
cargo test --workspace --all-features
cargo test -p <crate> test_name
cargo test --workspace -- --nocapture
cargo clippy --workspace --all-targets --all-features -- -D warnings  # CI uses -D warnings
cargo fmt --all
cargo fmt --all -- --check
cargo deny check
cargo doc --workspace --no-deps --all-features
cargo +nightly miri test -p <crate>

# Full CI check (run before pushing)
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features && cargo doc --workspace --no-deps && cargo deny check
```

</details>

### Add a New Public Type or Function

1. Add it in the appropriate crate under `crates/<name>/src/`. Respect the
   dependency chain — `mif-core` has no internal deps; `mif-schema` depends
   only on `mif-core`; `mif-ontology` depends on both; `mif-cli`/`mif-mcp`
   depend on whichever libraries they call, directly.
2. Annotate with `#[must_use]` if it returns a value without side effects.
3. Use `const fn` only where the compiler actually allows it — see "Why Not
   All Builders Are `const fn`" below; a struct-field reassignment that
   drops an `Option<String>` (or any type with a non-trivial `Drop`) cannot
   be `const`, even via `mut self`.
4. Write a doc comment: brief summary, `# Errors` (if fallible), `# Examples`
   where practical.
5. Add a unit test in the `#[cfg(test)] mod tests` block within the same file.
6. Run `just check` before committing.

### Add a New Error Variant

1. Add the variant to the relevant crate's error enum (`mif_schema::MifSchemaError`, `mif_ontology::OntologyError`), derived with `thiserror::Error`.
2. Include a `#[error("...")]` format string with meaningful context.
3. Prefer structured variants (named fields, e.g. `{ path: String, source: ... }`) over tuple variants when there are multiple pieces of context.
4. Add a test exercising the new failure path.

---

<!-- Diátaxis: Reference — precise, factual, information-oriented -->

## Reference

### Source Layout

| Path | Purpose |
|---|---|
| `crates/mif-core/src/{concept,entity,ontology}.rs` | `ConceptType`; `EntityReference`/`EntityId`/`EntityType`/`KnownEntityType`; `OntologyReference` |
| `crates/mif-schema/src/lib.rs` | Vendored-schema validators (`validate_document`, `validate_citation`, `validate_ontology_definition`) |
| `crates/mif-schema/src/schemas/` | Vendored copies of `mif.schema.json`, `citation.schema.json`, `ontology.schema.json`, `definitions/entity-reference.schema.json`, synced from the `MIF` repo's `schema/` |
| `crates/mif-ontology/src/lib.rs` | `OntologyMetadata`, `parse_definition`, `load_corpus_from_dir`, `resolve_chain` |
| `crates/mif-problem/src/lib.rs` | `ProblemDetails`, `Applicability`, `SuggestedFix`, `CodeAction`, `ProblemMeta`, `OutputFormat`, the `ToProblem` trait |
| `crates/mif-frontmatter/src/lib.rs` | `parse_markdown`, `serialize_markdown`, `md_to_jsonld`, `jsonld_to_md`, `roundtrip_lossless` |
| `crates/mif-embed/src/lib.rs` | `Embedder` (`load`, `embed`), `EMBEDDING_DIM` |
| `crates/mif-store/src/lib.rs` | `VectorStore` (`open`, `upsert`, `get`, `count`), `StoredVector` |
| `crates/mif-cli/src/main.rs`, `crates/mif-mcp/src/main.rs` | Thin binaries calling straight into the library crates' public functions |
| `clippy.toml` | Clippy thresholds and test-mode exemptions (workspace-root, applies to all members) |
| `rustfmt.toml` | Formatter settings (workspace-root) |
| `deny.toml` | Supply chain policy: licenses, bans, source restrictions (workspace-root) |
| `justfile` | Local task runner recipes (CI parity) |

### Error Handling

- Each library crate owns its own error enum, derived with `thiserror::Error` (`MifSchemaError`, `OntologyError`, `FrontmatterError`, `EmbedError`, `StoreError`). No shared top-level error type across the workspace — each crate's errors are scoped to what it actually does.
- **Propagation**: use `?`. Never `unwrap()`, `expect()`, or `panic!()` in library code (`crates/mif-core`, `mif-schema`, `mif-ontology`, `mif-problem`, `mif-frontmatter`, `mif-embed`, `mif-store`) — all are `deny`d workspace-wide via `[workspace.lints.clippy]`.
- **RFC 9457 Problem Details**: every library error enum implements `mif_problem::ToProblem` (`to_problem(&self) -> ProblemDetails`), mapping each variant to a stable, versioned problem-type URI via a per-variant `ProblemMeta` (see `mif-problem`'s doc comments for the pattern). `mif-cli`'s and `mif-mcp`'s own error enums (`CliError`, `McpError`) delegate to the wrapped library error's `to_problem()` for variants that wrap one, and define their own `ProblemMeta` only for binary-local variants (`Io`, `Json`).
- **`mif-cli`**: `main()` returns `ExitCode`, selects `mif_problem::OutputFormat` via an explicit `--format pretty|json` flag (falling back to stderr TTY detection), and renders errors with `error.render(format)` — pretty text or a compact `application/problem+json` envelope. Exempts itself from `print_stdout`/`print_stderr` via `#![allow(...)]` at the crate root (a CLI naturally needs to print — see "Lint Configuration" below).
- **`mif-mcp`**: `main()` returns `anyhow::Result<()>`, but its `#[tool]` methods return `String` values through the MCP protocol rather than printing. An MCP client is inherently a machine consumer, so every tool failure always renders as `error.to_problem().to_json()` (no pretty/JSON format choice) — it needs no `print_stdout`/`print_stderr` allow, since it never calls `println!`/`eprintln!`.

### Ownership and Borrowing

- Prefer `&str` over `String` in function parameters.
- Prefer `&[T]` over `Vec<T>` in function parameters.
- Pass large structs by reference; pass `Copy` types by value.
- Avoid unnecessary `.clone()` — if you need ownership, take owned types in the signature.

### Type Design

- Derive `Debug` on all types. Derive `Clone`, `PartialEq`, `Eq`, `Hash` when semantically correct (clippy's `derive_partial_eq_without_eq` is a hard error — if you derive `PartialEq`, derive `Eq` too whenever the fields allow it).
- Prefer `enum` for closed sets. `mif_core::EntityType` uses a `Known(..) | Custom(String)` pattern (via `#[serde(untagged)]`) for schema fields that are a closed enum *or* a pattern-matched custom string — this preserves round-trip fidelity for values the closed variant doesn't cover; don't use `#[serde(other)]` for this, it discards the original string.
- `HashMap`-accepting public functions should be generic over `S: std::hash::BuildHasher` (clippy's `implicit_hasher`), e.g. `mif_ontology::resolve_chain<S: BuildHasher>(id: &str, corpus: &HashMap<String, OntologyMetadata, S>)`.

### Builder Pattern

Consuming-self builders, matching this workspace's convention:

```rust
#[must_use]
pub fn with_field(mut self, value: T) -> Self {
    self.field = value;
    self
}
```

**Not always `const fn`.** Reassigning `self.field` when the field's type
has a non-trivial `Drop` (e.g. `Option<String>`, `Option<SomeEnumHoldingAString>`)
requires the compiler to drop the old value first, and `String`'s destructor
is not const-evaluable in stable Rust — this fails to compile as `const fn`
even though it looks identical to a builder over `Copy` fields. Fresh struct
*construction* (`Self { field: None, .. }` in a `new()`) has no old value to
drop and stays `const fn`-able; *mutation* of an existing `self`'s
String-bearing field does not. Mark `const` only where it actually compiles
— don't assume it from this pattern alone.

### Const and Must-Use Annotations

- `#[must_use]` on all pure functions that return a value.
- `const fn` wherever the compiler allows it (see caveat above).

### Lint Configuration

Set in the workspace root `Cargo.toml`'s `[workspace.lints]` (not per-crate); every member opts in via `[lints] workspace = true`. Clippy runs with **pedantic + nursery + cargo** lint groups, all `warn` priority -1.

**Denied lints** (hard errors):

| Lint | Reason |
|---|---|
| `unwrap_used` | Use `?` or explicit match |
| `expect_used` | Use `?` or explicit match |
| `panic` | Return errors instead |
| `todo` | No placeholder code |
| `unimplemented` | No placeholder code |
| `dbg_macro` | No debug prints in production |
| `print_stdout` | Use logging; binaries exempt themselves with `#![allow(...)]` at the crate root |
| `print_stderr` | Use logging; binaries exempt themselves with `#![allow(...)]` at the crate root |

**Allowed lints** (set explicitly in `[workspace.lints.clippy]`):

| Lint | Reason |
|---|---|
| `missing_errors_doc` | Opt-in documentation |
| `missing_panics_doc` | Opt-in documentation |
| `module_name_repetitions` | Common in Rust API design |
| `must_use_candidate` | Applied manually where meaningful |
| `redundant_pub_crate` | Allow `pub(crate)` for clarity |
| `multiple_crate_versions` | Inherent to a dependency graph pulling in `jsonschema`/`rmcp`/`tokio`; not a code-quality signal about this workspace's own code |

**Framework-imposed exceptions** (documented inline where used, not workspace-wide): `mif-mcp`'s `#[tool]`-annotated methods require an `&self` receiver for `rmcp`'s dispatch mechanism even when unused — `#[allow(clippy::unused_self)]` on that `impl` block, with a comment explaining why.

**Clippy thresholds** (from `clippy.toml`, workspace-root, applies to every member):

| Threshold | Value |
|---|---|
| `too-many-lines-threshold` | 100 |
| `too-many-arguments-threshold` | 7 |
| `cognitive-complexity-threshold` | 25 |
| `excessive-nesting-threshold` | 4 |
| `max-struct-bools` | 3 |
| `max-fn-params-bools` | 3 |
| `pass-by-value-size-limit` | 256 bytes |
| `type-complexity-threshold` | 250 |

**Test exemptions**: `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-dbg-in-tests`, `allow-print-in-tests` are all `true` — use plain `.unwrap()` in `#[cfg(test)]` code, not `.unwrap_or_default()` workarounds.

### Formatting

Configured in `rustfmt.toml` (workspace-root, stable options active):

| Setting | Value |
|---|---|
| `max_width` | 100 |
| `edition` | 2024 |
| `tab_spaces` | 4 |
| `hard_tabs` | false |
| `use_field_init_shorthand` | true |
| `reorder_imports` | true |
| `reorder_modules` | true |
| `newline_style` | Unix |
| `match_block_trailing_comma` | true |

### Import Ordering

Group imports in this order, separated by blank lines: `std`/`core`/`alloc`, external crates, `crate`/`super`/`self`. Alphabetical within each group (`reorder_imports = true`). rustfmt also alphabetizes `use rmcp::{A, B, C}`-style multi-imports — don't fight it.

### Doc Comments

All public items require doc comments (`missing_docs = "warn"` workspace-wide). Structure: brief one-line summary, extended description if needed, `# Errors` for fallible functions, `# Examples` where it adds value. Doc examples compile as doctests (`cargo test` runs them).

### Unsafe Code

`unsafe` code is **forbidden** (`unsafe_code = "forbid"` in `[workspace.lints.rust]`). No exceptions.

### Supply Chain Security

`deny.toml` (workspace-root) enforces:

- **Licenses**: permissive only (MIT, MIT-0, Apache-2.0, BSD-2/3, ISC, Zlib, MPL-2.0, Unicode, CC0, BSL-1.0, 0BSD). When a new dependency's license isn't in the allow-list yet, verify it's genuinely a safe permissive license before adding it (`cargo deny check licenses` names the exact crate/license) — don't blanket-allow.
- **Sources**: crates.io only; unknown registries and git sources denied.
- **Bans**: `openssl` (use `rustls`), `atty` (use `std::io::IsTerminal`).
- **Advisories**: all advisory types (vulnerability, unmaintained, unsound, notice, yanked) denied.
- **Wildcards**: wildcard version requirements denied.
- **Multiple versions**: set to `warn`, not `deny` — real-world dependency graphs (this one included: `hashbrown` 0.16 vs 0.17 via `jsonschema` vs `serde_norway`) routinely carry duplicate transitive versions that aren't worth pinning around.

**Dependency feature hygiene**: don't take a crate's default features blind. `jsonschema`'s defaults pull in a full `reqwest`/`rustls`/`aws-lc-rs` HTTP-resolver stack for `$ref` resolution this workspace doesn't use (all `$ref`s resolve offline via a custom `Registry` in `mif-schema`) — it's pinned `default-features = false` in `[workspace.dependencies]`. Check `cargo tree -i <suspicious-crate>` when a build pulls in something unexpected.

### Testing

| Test type | Location |
|---|---|
| Unit tests | `#[cfg(test)] mod tests` inside each crate's source files |
| Doc tests | `///` examples on public items |

**Property test pattern** (if adding `proptest` to a crate that needs it — not currently a workspace dependency):

```rust
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn my_property(input in any::<i32>()) {
            prop_assert!(some_invariant(input));
        }
    }
}
```

### CI/CD

CI runs through `pipeline.yml`, `ci-checks.yml`, and `quality-gates.yml`; releases run through tag-triggered `release.yml`, `publish.yml`, `package-homebrew.yml`. Every security-gate job across `quality-gates.yml` (`sast`, `sca`, `posture`, `trivy`) and `pipeline.yml` (`pin-check`, `validate-workflows`, `docker-sign`, `docker-verify`, `gate-image`, `attest-container-scan`) calls **`modeled-information-format/.github`**'s reusable workflow catalog, not `attested-delivery/.github` — this repo forked from `attested-delivery/rust-template`, but as a member of the `modeled-information-format` org it uses the org's own security-gate infrastructure, matching every other repo in this workspace's ecosystem. (The two files don't share job *names* beyond `reusable-trivy.yml`, called by both `quality-gates.yml`'s `trivy` job and `pipeline.yml`'s `gate-image` job.)

**Multi-crate, multi-binary, not single-package**: `publish.yml`'s guard/publish logic and `release.yml`/`package-homebrew.yml`'s binary-resolution logic are driven dynamically off `cargo metadata` (`.packages[] | select(...)`, never `.packages[0]`) so they scale to any number of workspace members and `[[bin]]` targets — both `mif-cli` and `mif-mcp` build on every release, and a future third binary crate needs zero workflow changes to join them. See `docs/runbooks/RELEASING.md` for the full procedure.

**`environment: release`** gates `publish.yml`/`release.yml`/`package-homebrew.yml` (renamed from the template's `copilot`) — configure real protection rules (required reviewer) on it in repo Settings before arming external publish channels.

Releases are orchestrated by the `/release` skill at **`.github/skills/release/SKILL.md`** (not `.claude/skills/` — that path doesn't exist in this repo). Artifact verification commands live in `SECURITY.md` § Verifying Release Artifacts.

### Cargo Profiles

Set once at the workspace root (`[profile.*]` — not member-level; profiles are workspace-only in a virtual manifest):

| Profile | Optimization | LTO | Codegen Units | Panic | Strip | Debug |
|---|---|---|---|---|---|---|
| `dev` | 0 | off | default | unwind | no | 1 (line tables) |
| `release` | 3 | thin | 1 | abort | yes | no |
| `release-debug` | 3 | thin | 1 | abort | no | full |

---

<!-- Diátaxis: Explanation — understanding-oriented, design rationale -->

## Explanation

### Why a Virtual Workspace, Not a Root Package

Five crates share a strict dependency chain (`mif-core` -> `mif-schema` -> `mif-ontology` -> `{mif-cli, mif-mcp}`) and are versioned/released together. A workspace gives real path dependencies during development, one shared `Cargo.lock`, and CI that catches a breaking `mif-core` change in the same PR that introduces it. The root manifest has no `[package]` section (a *virtual* workspace) since no code lives at the workspace root itself — every crate is a real member under `crates/`.

### Why the Libraries Don't Depend on the Binaries

`mif-cli` and `mif-mcp` are thin consumers of `mif-core`/`mif-schema`/`mif-ontology`'s public APIs, not the other way around. The three libraries are published independently and meant to be genuinely reusable by third parties who have no interest in a CLI or an MCP server — CLI/MCP-specific concerns (argument parsing, tool-schema derivation) stay out of the library layer entirely.

### Why `thiserror` for Errors

`thiserror` provides derive macros for `std::error::Error` with zero runtime overhead, generating `Display` and `From` implementations from attributes. Each library crate's error enum stays scoped to that crate's own failure modes rather than a shared top-level type, since `mif-schema` and `mif-ontology` fail in genuinely different ways (schema validation vs. corpus/graph resolution).

### Why Vendor the JSON Schema Instead of Fetching at Validate Time

A library doing an HTTP fetch on every validation call is non-deterministic and breaks offline/sandboxed CI. `mif-schema` embeds `mif.schema.json`, `citation.schema.json`, `ontology.schema.json`, and `definitions/entity-reference.schema.json` via `include_str!` and resolves `$ref`s through a custom `jsonschema::Registry` keyed by each file's own `$id` — no network access happens at validation time, and `jsonschema`'s default HTTP/file-resolver features are explicitly disabled.

### Why Generic Frontmatter Pass-Through, Not a Curated Field List

`mif_convert.py` (the canonical `MIF` repo's Python reference converter this crate ports) only recovers a fixed list of passthrough fields when projecting JSON-LD back to markdown, silently dropping any other frontmatter key on the full `md -> json-ld -> md` pipeline. `mif-frontmatter` deliberately does not reproduce that limitation: `md_to_jsonld`/`jsonld_to_md` pass every frontmatter/JSON-LD key through generically (`FRONTMATTER_ORDER` governs serialization *order* only, not which keys survive), since `mif.schema.json`'s root object doesn't set `additionalProperties: false` — unrecognized top-level keys are already spec-legal, so dropping them was a bug in the reference converter, not a behavior worth preserving. This was verified against real documents: `research-harness-template`'s own findings and Level-3 reports carry fields (`slug`, `version`, harness-specific `extensions.harness` data) the fixed Python list never anticipated, and previously failed `roundtrip_lossless` with `RoundTripDrift` until this crate stopped curating.

A second, genuinely irreducible ambiguity this surfaced: a document's `@id`/`conceptType` identity can be expressed either as MIF v1.0's `id`/`type` shorthand (projects to `@id: urn:mif:{id}`) or already-projected literal `@context`/`@type`/`@id`/`conceptType` frontmatter keys (e.g. `research-harness-template`'s reports) — both produce an identical `@id` string in JSON-LD, so there is no way to tell them apart from the JSON-LD value alone on the reverse trip. `mif_frontmatter::FrontmatterShape` (`V1Canonical` | `PreProjected`) makes this explicit: `md_to_jsonld` auto-detects it from the frontmatter it's given (a literal `@id` key present means `PreProjected`), while `jsonld_to_md` requires the caller to state it, since it has no frontmatter to inspect. `roundtrip_lossless` detects and threads it through internally; external callers converting standalone JSON-LD (no originating markdown) default to `V1Canonical`, the MIF v1.0 authoring convention.

### Why Hand-Written Types, Not Schema-to-Rust Codegen

`mif-core`'s four types (`OntologyReference`, `EntityReference`, `EntityData`, `ConceptType`) are hand-written and field-verified directly against the live schema, not generated via a tool like `typify`. The scoped 4-type surface is stable and low-drift-risk, and generic codegen doesn't naturally produce this workspace's idiomatic conventions (consuming-self builders, the closed-enum-or-custom `EntityType` fallback that preserves unknown values verbatim). Revisit codegen if/when a fuller document-type surface (a full `Mif` struct mirroring every optional field of `mif.schema.json`) gets built — that's the case where hand-maintenance drift risk would outweigh codegen's ergonomic cost.

### Why Pedantic Clippy

Enabling `pedantic`, `nursery`, and `cargo` lint groups catches subtle issues early: missing docs, inefficient patterns, cargo metadata problems. The strict deny list (`unwrap_used`, `panic`, etc.) enforces that library code handles all errors explicitly, pushing failures to the API boundary where callers can make decisions.

### Why `panic = "abort"` in Release

Release builds use `panic = "abort"` to eliminate unwinding tables, reducing binary size. Combined with `strip = true` and `lto = "thin"`, this produces small, fast binaries for both `mif-cli` and `mif-mcp`. The `release-debug` profile inherits these optimizations but preserves debug symbols for profiling.

### Why Ban `openssl` and `atty`

- **`openssl`**: links to a system C library with complex build requirements and CVE history. `rustls` is a pure-Rust TLS implementation with smaller attack surface.
- **`atty`**: unmaintained and unnecessary since Rust 1.70 added `std::io::IsTerminal` to the standard library.
