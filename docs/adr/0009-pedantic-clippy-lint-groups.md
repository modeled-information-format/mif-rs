---
title: "Pedantic, Nursery, and Cargo Clippy Lint Groups, Workspace-Wide"
description: "Enable clippy's pedantic, nursery, and cargo lint groups at warn priority across all 9 mif-rs workspace members, combined with a curated hard-deny set and a curated allow-list defined once in the workspace root."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: quality
tags:
  - adr
  - clippy
  - lints
  - quality
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0003-virtual-cargo-workspace.md
---

# ADR-0009: Pedantic, Nursery, and Cargo Clippy Lint Groups, Workspace-Wide

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs` is a 9-crate Cargo workspace (`mif-core`, `mif-schema`, `mif-ontology`,
`mif-problem`, `mif-frontmatter`, `mif-embed`, `mif-store`, `mif-cli`,
`mif-mcp`) implementing the MIF (Modeled Information Format) specification as a
library other crates and services depend on. The root `Cargo.toml`'s
`[workspace.lints.clippy]` table enables clippy's `all` group plus the
`pedantic`, `nursery`, and `cargo` groups, each at `warn` priority `-1`,
alongside a curated hard-deny set and a curated allow-list. The adjacent
`[workspace.lints.rust]` table separately sets `unsafe_code = "forbid"` and
`missing_docs = "warn"`. Every one of the 9 workspace members opts in via
`[lints]` / `workspace = true` in its own `Cargo.toml` rather than defining a
crate-local lint table. This ADR documents that decision.

### Current Limitations

1. **Plain clippy defaults under-catch for a spec-implementing library**:
   clippy's default lint set (`correctness` plus a small style tier) does not
   include `missing_docs`, `module_name_repetitions`, or the dozens of other
   pedantic-tier lints that matter specifically when the API surface itself —
   not just internal code health — is part of the deliverable for third-party
   reuse.
2. **No structural guard against internal panics**: without an explicit
   deny-list, `unwrap()`, `expect()`, `panic!()`, `todo!()`, and
   `unimplemented!()` can compile silently anywhere in library code, letting
   failures surface as process aborts instead of being pushed to the API
   boundary as `Result` values.
3. **Per-crate duplication risk**: with 9 members, a lint policy defined
   per-crate instead of once at the workspace root would drift as crates are
   added or as the policy is tuned.

## Decision Drivers

### Primary Decision Drivers

1. **Catch subtle issues early**: across a 9-crate workspace implementing a
   public specification meant for third-party reuse, the lint configuration
   shall catch missing documentation, inefficient patterns, and cargo
   metadata problems before they reach a published crate.
2. **No internal panics in library code**: library code shall handle all
   errors explicitly via `Result`, pushing failures to the API boundary rather
   than panicking internally.

### Secondary Decision Drivers

1. **Single source of truth**: the lint policy shall be defined once, at the
   workspace root, rather than duplicated or re-derived per crate.

## Considered Options

### Option 1: Clippy default lints only

**Description**: Rely on clippy's default lint set (the `correctness` group
plus a small built-in style tier), with no `pedantic`, `nursery`, or `cargo`
groups enabled.

**Advantages**:

- Zero configuration burden — this is clippy's out-of-the-box behavior, with
  no workspace-level lint table to write or maintain.
- No friction for new contributors: every stable Rust toolchain's default
  clippy invocation already matches this bar.

**Disadvantages**: Misses `missing_docs`, `module_name_repetitions`, and
dozens of other pedantic-tier catches that matter specifically for a
spec-implementing library meant for third-party reuse, where API surface
quality is part of the deliverable, not just internal code health.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: None.
- **Ecosystem Risk**: High. A published, spec-implementing crate with an
  under-linted public API surface erodes trust with third-party consumers.

### Option 2: pedantic + nursery + cargo at warn, with a curated deny-list and allow-list (chosen)

**Description**: Enable clippy's `pedantic`, `nursery`, and `cargo` lint
groups at `warn` priority `-1`, workspace-wide, combined with a small,
explicit hard-deny set for patterns that must never appear in library code,
and a curated allow-list for pedantic-tier lints judged to be genuine matters
of taste for this codebase rather than real defects.

**Advantages**:

- Surfaces the full pedantic/nursery/cargo lint surface across all 9 members
  from one workspace-level table, with no per-crate duplication.
- The hard-deny set (`unwrap_used`, `expect_used`, `panic`, `todo`,
  `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr`) makes it a
  compile error for library code to contain a stray panic or debug print.
- The allow-list keeps genuine matters of taste (e.g. `must_use_candidate`,
  `module_name_repetitions`) from generating warning noise, without silencing
  the rest of the pedantic tier.

**Disadvantages**:

- New contributors face a stricter bar than plain default clippy, with some
  friction until the allow-list's rationale is internalized.
- The deny-list and allow-list require ongoing curation as clippy's own lint
  set evolves.

**Risk Assessment**:

- **Technical Risk**: Low. `warn` priority means CI enforces the bar via
  `-D warnings` on the whole invocation, not via clippy's own defaults.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

### Option 3: pedantic + nursery + cargo with everything at deny, no allow-list

**Description**: Enable the same three groups, but at `deny` instead of
`warn`, with no workspace-level allow-list for any pedantic-tier lint.

**Advantages**:

- Maximally strict: every pedantic/nursery/cargo lint is a hard compile
  error, leaving no `-D warnings` CI flag to separately enforce the bar.
- No allow-list to curate or keep in sync as clippy's own lint set evolves.

**Disadvantages**: Several pedantic-tier lints are genuinely matters of taste
for this codebase (`must_use_candidate`, `module_name_repetitions`); denying
them outright would force constant `#[allow]` noise scattered through the
codebase instead of one curated, documented workspace-level allow-list.

**Disqualifying Factor**: judged too brittle — a single workspace-level
allow-list, reviewed and documented once, is preferable to scattered
per-call-site `#[allow]` annotations with no central record of why each one
exists.

**Risk Assessment**:

- **Technical Risk**: Medium. Denying subjective lints outright pushes the
  exemption burden onto scattered `#[allow]` annotations instead of one
  reviewed table.
- **Schedule Risk**: Medium. Contributors would hit CI failures on lints this
  codebase has already judged to be non-issues, slowing every PR touching
  those patterns.
- **Ecosystem Risk**: Low.

## Decision

We adopt **Option 2**: clippy's `pedantic`, `nursery`, and `cargo` lint groups
enabled at `warn` priority `-1`, workspace-wide, combined with a curated
hard-deny set and a curated allow-list.

The root `Cargo.toml`'s `[workspace.lints.clippy]` table currently reads:

```toml
[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
cargo = { level = "warn", priority = -1 }

# Specific lints to deny
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
print_stdout = "deny"
print_stderr = "deny"

# Allow in certain contexts (can be overridden per-crate)
missing_errors_doc = "allow"
missing_panics_doc = "allow"
module_name_repetitions = "allow"
must_use_candidate = "allow"
redundant_pub_crate = "allow"
multiple_crate_versions = "allow"
```

The adjacent `[workspace.lints.rust]` table sets `unsafe_code = "forbid"` and
`missing_docs = "warn"` (plus `rust_2024_compatibility` at `warn` priority
`-1`) — outside this ADR's clippy-specific scope, but part of the same
workspace-level lint policy.

Every one of the 9 workspace members opts in with:

```toml
[lints]
workspace = true
```

The deny-list and allow-list are **living lists**: expected to be adjusted as
new pedantic-tier lints are judged not to fit this codebase, or as lints
currently on the allow-list turn out to matter after all.

## Consequences

### Positive

1. **No stray panics compile**: library code cannot compile with a stray
   `unwrap()`, `expect()`, or `panic!()` outside `#[cfg(test)]` code —
   test-mode exemptions are handled separately via `clippy.toml`'s
   `allow-unwrap-in-tests` and similar settings, not by this deny-list.
2. **One consistent bar, no duplication**: a single workspace-level lint
   table applies the same code-quality bar across all 9 members, with no
   per-crate lint table to keep in sync.

### Negative

1. **Stricter bar for new contributors**: contributors face a stricter bar
   than plain default clippy, with some friction until the allow-list's
   rationale — which lints were deliberately judged not worth enforcing, and
   why — is internalized.

### Neutral

1. The allow-list is a living document, not a fixed, permanent exemption set
   — it is expected to be revisited as the codebase and clippy's own lint set
   evolve.

## Decision Outcome

The decision achieves its primary objective — a consistent, spec-appropriate
lint bar across all 9 workspace members — measured by: CI's
`cargo clippy --workspace --all-targets --all-features -- -D warnings`
passing with zero warnings across all 9 members, using exactly the deny/allow
configuration in the root `Cargo.toml`'s `[workspace.lints.clippy]` table
quoted above.

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace, Not a Root Package](0003-virtual-cargo-workspace.md) — establishes the 9-member workspace this lint policy applies across.

## Links

- [Clippy Documentation](https://doc.rust-lang.org/clippy/) — the official
  clippy book, including lint configuration and `clippy.toml` reference.
- [Clippy Lint Groups](https://doc.rust-lang.org/clippy/lints.html) — the
  full catalog of lints grouped by `correctness`, `style`, `complexity`,
  `perf`, `pedantic`, `nursery`, `cargo`, and `restriction`.
- [`unwrap_used` lint](https://rust-lang.github.io/rust-clippy/master/index.html#unwrap_used)
  — one of the hard-denied restriction-tier lints in this ADR's deny-list.
- [Cargo Workspace Lints (`[workspace.lints]`)](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-lints-table)
  — the Cargo manifest mechanism this ADR relies on to define the policy
  once at the workspace root.

## More Information

- **Date**: 2026-07-03
- **Source**: the root `Cargo.toml`'s `[workspace.lints]` tables (retroactively
  documents an established, ongoing configuration).

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| `[workspace.lints.clippy]` enables `all`/`pedantic`/`nursery`/`cargo` at warn priority -1, denies `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr`, and allows `missing_errors_doc`, `missing_panics_doc`, `module_name_repetitions`, `must_use_candidate`, `redundant_pub_crate`, `multiple_crate_versions` | Cargo.toml | 96-121 | accepted |

**Summary:** Decision adopted at workspace bootstrap and verified directly
against the current root `Cargo.toml` on 2026-07-03; no prior alternative
lint policy was in active use to migrate away from.

**Action Required:** None — this ADR documents current, already-adopted
practice.
