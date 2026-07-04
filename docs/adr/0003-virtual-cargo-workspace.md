---
title: "Virtual Cargo Workspace, Not a Root Package"
description: "Use a virtual Cargo workspace with no [package] section at the workspace root, and every one of mif-rs's 9 crates as a real member under crates/, rather than a root package that also carries library code."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - architecture
  - workspace
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0001-use-architectural-decision-records.md
---

# ADR-0003: Virtual Cargo Workspace, Not a Root Package

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs` is a 9-crate Cargo workspace (`mif-core`, `mif-problem`, `mif-schema`,
`mif-frontmatter`, `mif-ontology`, `mif-embed`, `mif-store`, `mif-cli`,
`mif-mcp`) with a strict dependency chain rooted at `mif-core`, fanning out
through `mif-schema`, `mif-ontology`, `mif-problem`, `mif-frontmatter`,
`mif-embed`, and `mif-store` to the two binaries `mif-cli` and `mif-mcp`. All
nine crates are versioned and released together. We need a workspace layout
that gives every crate a real path dependency on its upstream crates during
development, a single shared lockfile across all nine members, and a CI setup
where a breaking change to `mif-core` is caught in the same pull request that
introduces it — not later, in a downstream crate's own CI run.

### Current Limitations

1. **No existing workspace root convention decided**: at bootstrap, Cargo
   offers two shapes for a multi-crate workspace — a virtual workspace (root
   `Cargo.toml` has no `[package]` table, only `[workspace]` and shared
   tables) or a root package (the workspace root is itself a crate, with
   other members as path dependencies) — and nothing in this repository yet
   recorded which one `mif-rs` uses or why.
2. **Two binaries complicate a root-package shape**: `mif-cli` and `mif-mcp`
   are both top-level, independently useful binaries. A root-package layout
   privileges one crate's identity as "the workspace," which does not fit
   cleanly once there are two binaries that both need to be ordinary members.

## Decision Drivers

### Primary Decision Drivers

1. **Real path dependencies in development**: every crate shall depend on its
   upstream crates via `path =`, not a published version, so a change in
   `mif-core` is immediately visible to every downstream crate without
   publishing a release first.
2. **One shared lockfile**: all nine members shall resolve against a single
   `Cargo.lock`, so dependency versions cannot silently drift between crates
   within the same workspace.
3. **Same-PR breakage detection**: CI shall catch a breaking change to
   `mif-core` in the same pull request that introduces it, not in a
   downstream crate's separate CI run days or releases later.

### Secondary Decision Drivers

1. **No privileged crate identity**: with two independently useful binaries
   (`mif-cli`, `mif-mcp`), the workspace layout should not force one crate to
   double as "the" workspace root — every crate, binary or library, should be
   an ordinary member.
2. **Contributor onboarding clarity**: a new contributor reading the root
   `Cargo.toml` should see only workspace-level configuration
   (`[workspace]`, shared lints, shared profiles), not a mix of workspace
   config and one particular crate's own manifest fields.
3. **Avoiding future restructuring cost**: the chosen shape should not need
   to be revisited if a third binary crate is added later.

## Considered Options

### Option 1: Single crate, everything in one lib.rs/main.rs

**Description**: Collapse all functionality into one crate, with a single
`lib.rs` and `main.rs` rather than nine separate crates.

**Advantages**:

- Simplest possible initial setup — no workspace configuration, no
  `path =` dependencies, no shared lint/profile tables to wire up.

**Disadvantages**:

- No clean library/binary separation — `mif-core`, `mif-schema`, and the
  other library crates could not be published or versioned independently of
  the binaries.
- Would violate the intended layering, where library crates never depend on
  the binaries — a single crate has no mechanism to enforce that boundary.

**Risk Assessment**:

- **Technical Risk**: High. No enforced layering; any module can reach into
  any other, defeating the whole point of the dependency chain.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: High. Cannot publish `mif-core` (or any other library
  crate) to crates.io independently of the binaries.

### Option 2: A root package with library code at the workspace root

**Description**: Give the workspace root `Cargo.toml` its own `[package]`
table carrying library code (for example, `mif-core`'s code living directly
at the workspace root), with the remaining crates as path-dependency members
alongside it.

**Advantages**:

- Still allows real `path =` dependencies between the root package and every
  other member, satisfying the real-path-dependencies driver.

**Disadvantages**:

- Conflates the workspace root's identity with one particular crate — the
  root `Cargo.toml` is no longer purely workspace-level configuration, it is
  also a crate manifest.
- Becomes awkward once there are two binaries (`mif-cli`, `mif-mcp`) that
  both need to be top-level members; neither can be "the" root package
  without arbitrarily privileging one over the other.

**Risk Assessment**:

- **Technical Risk**: Medium. Works for a single-binary workspace but does
  not generalize to `mif-rs`'s two binaries.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Medium. A future third binary or a rename of the
  root-package crate would force a restructuring the virtual-workspace shape
  never needs.

### Option 3: Pure virtual workspace (chosen)

**Description**: No `[package]` section at the workspace root. Every crate —
`mif-core`, `mif-problem`, `mif-schema`, `mif-frontmatter`, `mif-ontology`,
`mif-embed`, `mif-store`, `mif-cli`, `mif-mcp` — is a real member under
`crates/`. Shared lint tables and release profiles are set once, at
`[workspace.lints]` and `[profile.*]` in the root `Cargo.toml`, and each
member opts in with its own `[lints]` `workspace = true`.

**Advantages**:

- Real path dependencies between members during development, so a change in
  `mif-core` is immediately visible downstream without publishing first.
- One `Cargo.lock` for the entire workspace.
- CI catches cross-crate breakage in the same pull request that introduces
  it, since `cargo build --workspace` / `cargo test --workspace` resolves
  and builds all nine members together.
- No crate is privileged as "the" workspace root — `mif-cli` and `mif-mcp`
  are ordinary members like every library crate.

**Disadvantages**:

- No code can live at the workspace root itself; everything must be a named
  member crate under `crates/`, even something that might feel like "just a
  helper module."

**Risk Assessment**:

- **Technical Risk**: Low. This is Cargo's standard multi-crate workspace
  shape.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

## Decision

We use a **pure virtual Cargo workspace**: the root `Cargo.toml` has no
`[package]` section, only `[workspace]`, `[workspace.dependencies]`,
`[workspace.lints]`, and `[profile.*]` tables. All nine crates — `mif-core`,
`mif-problem`, `mif-schema`, `mif-frontmatter`, `mif-ontology`, `mif-embed`,
`mif-store`, `mif-cli`, `mif-mcp` — are real members under `crates/`, each
depending on its upstream crates via `path =` through
`[workspace.dependencies]`, and each inheriting the shared lint and profile
tables via its own `[lints]` `workspace = true`.

## Consequences

### Positive

1. **Real path dependencies in development**: crates iterate against each
   other's current source, not a published version, so cross-crate changes
   are visible immediately without waiting on a publish.
2. **One `Cargo.lock`**: a single lockfile governs dependency resolution for
   the whole workspace, so no member can silently drift onto a different
   version of a shared dependency.
3. **Same-PR breakage detection**: CI catches a breaking change to
   `mif-core` (or any upstream crate) in the same pull request that
   introduces it, since `cargo build --workspace` / `cargo test --workspace`
   builds and tests all nine members together.

### Negative

1. **No workspace-root code**: everything must be a named member crate under
   `crates/`, even for something that might feel like "just a helper
   module." At this workspace's current scale (nine crates, a strict layered
   dependency chain) this is minor friction, not a real cost.

### Neutral

1. The root `Cargo.toml` carries only workspace-level concerns —
   `[workspace]`, `[workspace.dependencies]`, `[workspace.lints]`,
   `[profile.*]` — no crate-specific configuration lives there.

## Decision Outcome

The decision achieves its primary objective — real path dependencies, one
shared lockfile, and same-PR breakage detection — measured by: every one of
the nine crates builds and tests via a single `cargo build --workspace` /
`cargo test --workspace` invocation, sharing one `Cargo.lock` and one set of
lint/profile tables, with no crate needing its own duplicated lint or profile
configuration.

## Related Decisions

- [ADR-0001: Use Architectural Decision Records](https://modeled-information-format.github.io/mif-rs/adr/0001-use-architectural-decision-records/) — establishes the ADR practice this document follows.

## Links

- [Cargo Book: Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html) - Canonical reference for virtual vs. root-package workspace shapes.
- [Cargo Book: `[workspace.lints]`](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-lints-table) - How a virtual workspace centralizes lint configuration for members to inherit.
- [Cargo Book: Profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) - How `[profile.*]` tables are set once at the workspace root and shared by all members.

## More Information

- **Date**: 2026-07-03
- **Source**: workspace bootstrap (see this repository's initial commits,
  e.g. "Bootstrap mif-rs: 5-crate workspace, multi-binary CI/release
  pipeline"). This ADR retroactively documents a decision made at that
  bootstrap, 2026-07-02.

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Root `Cargo.toml` has no `[package]` section and lists all 9 members under `crates/`, confirmed by direct read | Cargo.toml | 1-13 | accepted |

**Summary:** The virtual-workspace structure was adopted at bootstrap and is
the codebase's current, unchanged state.

**Action Required:** None — this ADR documents current, already-adopted practice.
