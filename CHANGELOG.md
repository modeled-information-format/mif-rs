# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **`mif-rh`**: `rewrite_doc_version` (used by `bump_version`'s family-doc updater) no longer deletes everything after the version on a `**Version:**` row — trailing cells like `| **Kind:** methodology` are preserved; only the leading semver token is replaced (#104).
- **`mif-rh`**: `read_source_content` now takes `content: Option<&str>` instead of `content: &str` — an explicitly provided value, including an empty one, is taken as the content and refused immediately with `EmptySourceContent` when blank, instead of falling through to a stdin read that blocks forever on a pipe that never reaches EOF. **Breaking** for direct callers of this published function: wrap a previously-passed `&str` in `Some(..)`, and pass `None` (not `""`) to opt into the stdin fallback (#105).

## [0.8.0] - 2026-07-13

### Added

- **`mif-store`**: `multi_root_top_k_similar`, `multi_root_get`, and `multi_root_stats` free functions (plus `RootedMatch`/`MultiRootStats` result types) let a query span a project-local vector store root and any number of additional roots, merge-ranked by cosine similarity. Each additional root fails closed if it cannot be opened or queried; a not-yet-created root is treated as empty.
- **`mif-cli`**: `search`, `find-similar`, and `corpus-stats` gain a repeatable `--extra-db-path` flag for multi-root queries; single-root behavior is unchanged, and multi-root output additionally shows which root each match/row came from.
- **`mif-mcp`**: `search_documents`, `find_similar_documents`, and `corpus_stats` gain a matching `extra_db_paths` array parameter, kept in lockstep with `mif-cli`; single-root JSON output is byte-for-byte unchanged (#99).

### Fixed

- **`mif-rh`**: `render_report`/`render_blog`/`render_book` no longer silently drop an artifact's `subtitle` field — it now projects into the `report` channel's `description` frontmatter and renders as a lede blockquote under the H1 in the `blog`/`book` channels (#81, #85).

### Documentation

- Corrected stale crate/tool counts in `AGENTS.md` and `CLAUDE.md` (workspace has grown to 12 crates; `mif-mcp` now exposes nine tools, not six) (#100).

## [0.7.0] - 2026-07-11

### Added

- **`mif-rh`**: `reconcile_changelog_links(root, check)` reconciles a harness `CHANGELOG.md`'s dated-section heading brackets and footer compare-links against real `v*` git tags — a section is bracketed with a compare-link once a real tag exists for it (or it's still awaiting its first release), and loses both once superseded by a later tagged release without ever getting its own tag. Repo URL for generated links is parsed from the existing `[Unreleased]` footer entry rather than hardcoded, so it works against any harness instance's actual remote (research-harness-template#397).
- **`mif-rh-cli`**: new `harness reconcile-changelog-links [--check] [--root <path>]` subcommand exposing the above.

## [0.6.1] - 2026-07-10

### Fixed

- **`mif-rh`**: `falsify()`'s fixture entries that supply an explicit verdict but omit `attempted_at` now default to the injected wall-clock `now` instead of a fixed `1970-01-01T00:00:00Z` placeholder — the old default read as maximally stale to freshness projections and was nonsensical audit-trail provenance on its own terms. `falsify()`'s original 2-argument public signature is preserved (delegating to `chrono::Utc::now()`); a new `falsify_with_now()` takes the injected clock explicitly for deterministic callers and tests (research-harness-template#359, #73).

## [0.6.0] - 2026-07-09

### Added

- **`mif-rh-cli ontology fetch --refresh`**: an id already pinned in `ontologies.lock.json` at a version different from the registry's current one is now left untouched, with a warning naming the drift, unless `--refresh` is passed to advance it deliberately. Previously `fetch` always vendored the registry's current version unconditionally, silently overwriting an existing pin (research-harness-template#270, mif-rs#60, #62).
- **`mif-rh-cli ontology check-pin-safety [ids...]`**: for each pinned ontology id the registry has since moved past, diffs the vendored and registry schema's required-field lists per entity type and warns only when a stamped finding is actually missing a newly required field — narrower than the plain version-drift warning `fetch` already emits, closing research-harness-template#270's proposed fix #2 (research-harness-template#270, mif-rs#61, #67).
- **`mif-cli validate --level 1|2|3`** (and `validate_mif_document`'s matching `level` MCP parameter): an L1/L2/L3 MIF level-floor overlay on top of core-schema validation — L2 additionally requires `namespace`/`modified`/`temporal`, L3 additionally requires `provenance` and a non-null `temporal.validFrom` (#40, #65).
- **`mif-cli roundtrip`/`emit-jsonld`/`emit-markdown`** (and matching `mif-mcp` tools `roundtrip_mif_document`/`emit_jsonld_document`/`emit_markdown_document`): standalone, pure markdown<->JSON-LD conversion and round-trip-fidelity proof, needing no database or embedding model (#41, #66).

### Fixed

- **`ontology fetch`**: the mutated `index_sha256`/`source` trust-root fields were not persisted to `ontologies.lock.json` when every requested id was left pinned-and-skipped (no per-id write occurred to do it), silently dropping a deliberate re-pin in that case.
- **`mif-rh-cli`/`mif-cli` free-text args**: `synthesize-corpus --preserved-insights`, `wrap-source --title`/`--content`, `suggest-type`'s `TEXT` positional, and `search`'s `QUERY` positional now accept a value starting with `-` (e.g. an authored Markdown bullet), instead of clap misparsing it as an unrecognized flag (#69, #70, #71).

### Changed

- **`sysinfo` dependency**: relaxed the exact `=0.38.4` pin, whose comment claimed `sysinfo >=0.38` needed a newer Rust than this workspace's MSRV — disproven, since a Dependabot bump to 0.38.4 already passed the MSRV Check CI job — to `0.38.4` (Cargo's caret range, `>=0.38.4, <0.39.0`), allowing patch updates (#56, #64).

## [0.5.0] - 2026-07-07

### Added

- **`mif-rh-cli ontology` subcommand surface** (`fetch`, `sync`, `lock-check`, `sync-registry`, `author`): on-demand ontology pack vendoring, cataloging, lock-file drift checking, registry sync, and drafting a new ontology from expansion clusters or a topic's ontology map — the compiled-engine replacement for research-harness-template's `fetch-ontology.sh`, `sync-packs.sh`, `check-ontology-lock.sh`, `sync-registry-ontologies.sh`, and `author-ontology.sh` (research-harness-template#276, Story #277, mif-rs#58).
- **`mif-rh-cli harness` subcommand surface**: corpus/concordance synthesis (`build-concordance`, `reconcile-session`, `topic-metadata`, `synthesize-corpus`, `import-corpus`, `synthesize-artifact`), validation gates (`assert-graph-mif`, `check-citation-integrity`, `check-shippable-typing`, `falsify`, `check-relationship-targets`, `validate-concordance`, `check-ontology-registry`), graph/index/membership/rendering (`build-graph`, `build-index`, `resolve-membership`, `render-artifact`), versioning/release orchestration (`goal-version`, `bump-version`, `check-version-bump`, `mif-project`), and feature toggles/packaging (`site-toggle-primary`, `site-toggle-plugin`, `pack-toggle`, `wrap-source`) — the compiled-engine replacement for the remaining jq-based scripts audited in research-harness-template#274 (Stories #282, #287, #293, #298, #302, mif-rs#58).

### Fixed

- **`mif-rh-cli` Windows crash**: the growing `HarnessCommand` subcommand enum's clap-derive expansion overflowed Windows' small default main-thread stack even for a trivial invocation; the real work now runs on a thread with an explicit larger stack, matching the standard fix for this class of issue.

### Changed

- **MSRV bumped from 1.92 to 1.95**, a potentially breaking change for consumers on an older toolchain. Unblocks taking `rusqlite` 0.40.1 (previously pinned to `=0.39.0`), whose transitive `libsqlite3-sys` 0.38.1 dependency requires the `cfg_select!` macro, stable since Rust 1.95 (#55).

## [0.4.0] - 2026-07-06

### Added

- **`mif-rh-cli calibrate --confusions`**: exports a ranked confusion matrix (gold/top1/count/finding_ids) from a calibration run, the grounding input for human-curating MIF ADR-020's `negative_examples` field (#42).
- **`negative_examples` scoring**: the `negative-demotion-v1` policy-gated demotion gate scores curated `negative_examples` in the candidate pipeline (`mif-ontology::confidence::negative_demotes`, `mif-rh::suggest`). A candidate whose query similarity to any curated negative example meets or exceeds its positive score is barred from tier 1; demotion never reorders candidate ranking, only gates its confidence tier (#43).

### Fixed

- **CI**: the workspace `cargo publish` retries until the publish plan drains, instead of failing on transient registry propagation delays (#33).

### Fixed

- **Container chain: central signer pin bumped** — v0.3.0's four image
  attest legs died in the pinned central `sign-and-attest` workflow,
  whose SBOM action attempted a release attach under the caller's
  read-only token; the org fixed the central workflow and this release
  bumps the five pinned references to the fixed SHA. Source code is
  identical to v0.3.0. The v0.3.0 GitHub release and crates.io channels
  are fully attested; its container images remain unattested because
  tag-triggered runs are locked to the workflow content at the tag.


## [0.3.0] - 2026-07-04

### Added

- **mif-rh**: harness-ontology engine crate family (#20-#29), the Rust
  implementation of rht's ontology classification pipeline:
  - `mif-rh`: deterministic resolve/review classification with byte-parity
    against rht's bash implementation (fail-closed parity gate in CI against
    a pinned rht checkout), SQLite finding index with semantic search, a
    tier-2 suggestion queue, tier-3 miss recording with mutual-similarity
    clustering, and `stamped-quantile-v1` threshold calibration
  - `mif-rh-cli`: `resolve`, `review` (`--suggest`, `--strict`,
    `--followup`), `suggest-type`, `calibrate`, `expansion-candidates`,
    plus binary-level parity tests and a Windows relationship-script
    failure-path test
  - `mif-rh-mcp`: MCP server exposing the engine read-only, with explicit
    problem+json errors (`corpus_stats` now fails loudly on a missing
    reports directory instead of returning zeroes)
- **mif-ontology**: confidence-tiered entity-type classification capability
  (MIF ADR-020): `EntityType` with `aliases`/`exemplars`/
  `negative_examples` and `embedding_doc()`, `ConfidenceTier`,
  `CalibrationConfig` (recalibratable artifact, fail-closed
  `CalibrationInvalid`), `assign_tier` (TAC-KBP floor+margin gate),
  `SimilarityBand`, and mutual-similarity clustering
- **mif-embed**: public `MODEL_ID` constant naming the pinned embedding
  model
- M2 review-performance benchmark harness (`just bench-review`; 4,354
  findings in 2.02 s seeded result) and the M3 cross-topic search eval
  over rht's known-similar-pairs fixture, both wired fail-closed in CI
- ADRs 0019 (mif-rh crates packaged in this workspace) and 0020
  (mif-rh-mcp stdio-only transport)

## [0.2.0] - 2026-07-04

### Added

- **ingest**: MIF document ingestion, embedding, and semantic search pipeline
  (#6), spanning four new crates:
  - `mif-problem`: shared RFC 9457 Problem Details envelope (`ProblemDetails`,
    `ToProblem` trait, `OutputFormat`), adopted workspace-wide across every
    crate's error enum — `--format pretty|json` on `mif-cli`, always-JSON on
    `mif-mcp`
  - `mif-frontmatter`: markdown-frontmatter <-> JSON-LD projection, generalized
    to a full generic field pass-through (`FrontmatterShape` distinguishes the
    v1.0 `id`/`type` shorthand from already-`@id`-shaped frontmatter)
  - `mif-embed`: local, offline-after-first-fetch sentence embeddings via
    `candle` (`sentence-transformers/all-MiniLM-L6-v2`)
  - `mif-store`: SQLite vector store (`rusqlite`, bundled) with
    brute-force cosine-similarity ranking
- **cli**: `mif-cli` gains `ingest`, `search`, `find-similar`, and
  `corpus-stats` subcommands
- **mcp**: `mif-mcp` gains matching `ingest_mif_document`, `search_documents`,
  `find_similar_documents`, and `corpus_stats` tools
- **hooks**: Add `lefthook.yml` git hooks mirroring this repo's own CI (#9) —
  pre-commit runs `cargo fmt --all -- --check`; pre-push runs the full CI
  parity sequence (fmt, clippy `-D warnings`, test, doc, `cargo deny check`).
  Run `lefthook install` after cloning.

### Fixed

- **ci**: Bump the `reusable-trivy.yml` pin to pick up a Trivy CLI version fix
  (v0.72.0) after Trivy 0.70.0 crashed scanning a malformed CVE-2026-6791
  record in the live `trivy-db` (#7)
- **trivy**: Add a `.trivyignore` entry for CVE-2026-6791, an unfixed glibc
  vulnerability in the Chainguard `glibc-dynamic` base image with no available
  fixed build yet, verified against the real published image (#8)

## [0.1.2] - 2026-06-24

### Added

- **errors**: Dual-consumer (human + LLM-agent) error output following the RFC 9457 Problem Details model
  - `ProblemDetails` serializable envelope carrying the five standard members (`type`, `title`, `status`, `detail`, `instance`) plus the `retry_after`, `suggested_fix`, and `code_actions` agent extensions and an optional `exit_code`
  - `Applicability` markers (`machine_applicable` / `maybe_incorrect` / `has_placeholders` / `unspecified`) on every suggested fix and code action
  - `Error::to_problem()` maps each variant to a distinct, version-embedded type URI derived from the configurable `ERROR_TYPE_BASE_URI`; the occurrence `instance` URN tracks the crate name
  - `OutputFormat` + `Error::render()` dual renderer; the binary selects JSON vs pretty via `--format` and stderr `IsTerminal` detection (pretty output is byte-identical to the prior `Error: {e}` line)
  - Per-type problem documentation under `docs/reference/errors/` (dereferenceable type URIs) and a "Dual-Consumer Error Output" explanation doc
- **fuzz**: Add a minimal cargo-fuzz harness targeting the public `process()` parser
- **docs**: Add an end-to-end "Attested Delivery" guide — which `modeled-information-format/.github` reusables run, the `publish` gate, the build → sign → verify chain, and a downstream adoption runbook
- **docs-site**: Add Astro Starlight documentation site at `site/`
  - Browsable, searchable pages deployed to GitHub Pages
  - Auto-generated content from `docs/` markdown, `.github/workflows/*.yml`, and `CLAUDE.md` reference sections
  - Embedded rustdoc API reference at `/api/`
  - Pagefind full-text search, Mermaid diagram support, OG/Twitter social meta
  - Content generation scripts with freshness checking (`npm run check:freshness`)
  - Splash landing page with feature cards and sidebar navigation
- **workflows**: Add `docs-freshness.md` gh-aw workflow for weekly staleness detection
- **ci**: Add template-init workflow for automatic repo renaming
- Add ADR validation and viewer workflows
- Add production-ready CI/CD and deployment workflows
- Add security & quality workflows with comprehensive docs
- Add comprehensive testing enhancements
- Add packaging & distribution for all major platforms
- Add UX enhancements and automation workflows
- Add advanced security and observability features
- Add community and governance files
- Add editor, devcontainer, and VS Code configuration
- Add GitHub config, Copilot setup, and CodeQL workflow
- Add documentation structure and ADR-0002
- Add justfile for local CI parity
- **commands**: Add `/spec-orchestrator` slash command for parallel agent team orchestration
  - Phase-based workflow: bootstrap, discovery, synthesis, execution, verification, cleanup
  - `jq`-based inventory processing to conserve agent context windows
  - Just-in-time teammate spawning with staleness prevention and heartbeat monitoring
  - Anti-takeover rules preventing the orchestrator from writing code itself
  - Mnemonic blackboard storage for persistent, project-isolated work directory
- **commands**: Add `/init-project` toolchain verification (Phase 1.5) requiring rustup over Homebrew
- Add `template-sync` recipe to justfile for syncing shared tooling from upstream

### Changed

- **ci**: Decouple the GitHub Release from the external-publish gate — any pushed tag now produces an attested GitHub Release (binaries + SBOM + source snapshot); `publish = false` gates only crates.io, the container image, and Homebrew
- **release**: Align the `/release` skill with the current attested workflows (source-snapshot and gate-attestation jobs; 8 release assets)
- **workflows**: Replace rustdoc+mdBook docs-deploy workflow with Astro Starlight site deployment
  - Builds Node.js site alongside rustdoc, embeds API docs at `/api/`
  - Triggers on `docs/**`, `site/**`, `CLAUDE.md`, and `Cargo.toml` changes

### Security

- **deps**: Force `esbuild` >= 0.28.1 in the docs site (GHSA-gv7w-rqvm-qjhr RCE via `NPM_CONFIG_REGISTRY`, GHSA-g7r4-m6w7-qqqr Windows dev-server path traversal)
- **docker**: Pin Dockerfile base images by digest (OpenSSF Scorecard Pinned-Dependencies, Trivy DS-0001) and add a `docker` Dependabot ecosystem to keep them fresh
- **ci**: Harden GitHub Actions token permissions to least privilege across all workflows (OpenSSF Scorecard Token-Permissions)
- **ci**: Scope the Trivy supply-chain scan away from the dev-only docs site, clearing license-classification noise from the code-scanning hub

### Build

- Add `serde` and `serde_json` runtime dependencies for JSON envelope serialization
- Bump thiserror 2.0.18 and proptest 1.10.0
- Bump taiki-e/install-action to v2.67.25

### CI/CD

- Use GitHub API for signed changelog commits
- Consolidate CI/release into unified pipeline
- Disable Docker Hub and crates.io publish triggers

### Documentation

- Rewrite Copilot Jumpstart prompts for 500-char limit
- Update project docs, rustfmt config, and tests
- Add commit signing guidance for contributors
- Add rustup toolchain setup guidance to GETTING-STARTED.md, README.md, and CONTRIBUTING.md (not Homebrew)
- Add 90% code coverage requirement across all metrics to CLAUDE.md
- Update documentation to reflect current codebase
- Add comprehensive deployment guide
- Add Copilot Jumpstart prompts for template users

### Fixed

- **docs**: Fix the GitHub Pages base path so the published site renders styled (assets were 404ing) and base-prefix internal navigation links
- **docs**: Align workflow-reference coverage with the actual workflows (remove stale reference pages) and regenerate all pages so committed content matches the generators
- Rename copilot-setup-steps job ID
- Add cargo deny check and rustls constraints to jumpstart prompts
- **workflows**: Correct SHAs, disable heavy triggers, fix SLSA structure
- **docs**: Add backticks to x86_64 in README for clippy doc_markdown lint
- **docker**: Keep Cargo.lock in Docker context and fix FROM casing
- **ci**: Correct git-cliff-action SHA in release and changelog workflows
- **ci**: Fix release asset upload and ARM64 strip
- **ci**: Rename binaries to unique asset names before upload
- **ci**: Add shell: bash to release upload step for Windows compat

### Refactored

- Rename src directory to crates

## [0.1.0] - 2026-02-07

### Added

- Update mif-rs
- Add Claude Code agents for development workflow

### CI/CD

- Add dependabot auto-merge workflow
- Update MSRV check to Rust 1.92

### Documentation

- Add MIT LICENSE file
- Fix LICENSE links in README for rustdoc
- Update copilot-instructions.md

### Fixed

- Update deny.toml to cargo-deny v2 format
- Update dtolnay/rust-toolchain action to v1
- Restore commit SHA pinning for rust-toolchain action

### Miscellaneous

- Update GitHub Actions and dependencies to December 2025 latest
- Update Rust dependencies to December 2025 latest versions
- **deps**: Bump the github-actions group with 2 updates
- **deps**: Bump taiki-e/install-action in the github-actions group
- **deps**: Bump taiki-e/install-action in the github-actions group
- **deps**: Bump actions/cache from 4.2.3 to 5.0.2
- **deps**: Bump taiki-e/install-action in the github-actions group
- **deps**: Bump actions/checkout from 4.2.2 to 6.0.2 (#15)
- **deps**: Bump taiki-e/install-action in the github-actions group (#14)
- **deps**: Bump the github-actions group with 2 updates (#16)

### Refactored

- Simplify code and update to Rust 1.92 best practices

<!-- generated by git-cliff -->
