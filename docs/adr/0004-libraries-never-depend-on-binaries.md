---
title: "Library Crates Never Depend on the Binary Crates"
description: "Keep mif-cli and mif-mcp as thin, one-directional consumers of the seven mif-rs library crates so those libraries remain standalone, publishable, and reusable by third parties with no interest in a CLI or an MCP server."
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
  - 0003-virtual-cargo-workspace.md
---

# ADR-0004: Library Crates Never Depend on the Binary Crates

## Status

Accepted

## Context

### Background and Problem Statement

`mif-cli` (a binary) and `mif-mcp` (a binary, an MCP server) are thin
consumers of seven library crates' public APIs: `mif-core`, `mif-schema`,
`mif-ontology`, `mif-problem`, `mif-frontmatter`, `mif-embed`, and
`mif-store`. Argument parsing (via `clap`) lives only in `mif-cli`; MCP
tool-schema derivation (via `rmcp`'s macros) lives only in `mif-mcp`. The
seven libraries are published independently and meant to be genuinely
reusable by third parties who have no interest in a CLI or an MCP server.

As the two binaries grow, there is a recurring temptation to lift shared
logic between them up into one of the libraries so both can reuse it,
instead of duplicating it. Left unexamined, that temptation would let
binary-only concerns leak into a library's public API.

### Current Limitations

1. **No stated rule against library-to-binary leakage**: nothing yet
   documents that a library crate must never depend on `mif-cli` or
   `mif-mcp`, so a future contributor (human or agent) could introduce such
   a dependency without recognizing it as a regression.
2. **Real duplication pressure**: `mif-cli` and `mif-mcp` each need their own
   test helper to guard against a concurrent-test race when warming the
   embedding model cache, which creates pressure to consolidate that helper
   somewhere shared.

## Decision Drivers

### Primary Decision Drivers

1. **Standalone reusability**: the seven library crates shall remain
   independently publishable to crates.io, usable by a third party with no
   interest in a CLI or an MCP server.
2. **No app-layer leakage**: CLI-specific concerns (argument parsing via
   `clap`) and MCP-specific concerns (tool-schema derivation via `rmcp`'s
   macros) shall never appear in a library crate's public API.

### Secondary Decision Drivers

1. **Minimal duplication overhead**: whatever accepted duplication remains
   between `mif-cli` and `mif-mcp` (for example, each binary's own
   `warm_embedding_model_cache` test helper) should stay small enough that
   maintaining it twice is cheaper than the ecosystem risk of sharing it
   through a library.
2. **Low verification cost**: the chosen direction should be checkable by
   reading each crate's `Cargo.toml`, not by auditing runtime behavior.

## Considered Options

### Option 1: Let a shared helper live in a library crate

**Description**: Move a helper that both binaries need — for example,
`mif-cli`'s argument-parsing types — into `mif-core` so `mif-mcp` can reuse
it too.

**Advantages**:

- Eliminates the literal duplication between `mif-cli` and `mif-mcp` for
  whatever helper gets moved.
- No new workspace member to introduce or maintain.

**Disadvantages**: Couples a library crate to a binary-only concern
(`clap` derives), polluting that library's public API surface for every
third-party consumer who has no interest in a CLI.

**Risk Assessment**:

- **Technical Risk**: Medium. Once a binary-only type ships in a library's
  public API, removing it later is a breaking change for every downstream
  consumer of that library.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: High. Third parties depending on the library for its
  actual domain logic inherit a `clap` dependency and API surface they never
  asked for.

### Option 2: Strict one-directional dependency (chosen)

**Description**: `mif-cli` and `mif-mcp` depend on whichever of the seven
libraries they call directly. None of the seven libraries depend on either
binary, ever.

**Advantages**:

- The seven libraries stay genuinely standalone and reusable by a third
  party with zero interest in a CLI or MCP server.
- CLI/argument-parsing and MCP-schema concerns stay confined to the two
  binaries that actually need them.
- Adding a third binary consumer in the future requires zero changes to any
  of the seven libraries.

**Disadvantages**:

- `mif-cli` and `mif-mcp` carry some literal duplication of logic that
  cannot be shared without a library-external helper crate.

**Risk Assessment**:

- **Technical Risk**: Low. The dependency direction is easy to verify by
  reading each crate's `Cargo.toml`.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

### Option 3: A shared third "mif-app" crate

**Description**: Merge `mif-cli`'s and `mif-mcp`'s shared logic into a new
`mif-app` crate that the seven libraries also depend on.

**Advantages**:

- Removes the literal duplication between `mif-cli` and `mif-mcp` by giving
  both a single shared home for the logic they need.

**Disadvantages**: Creates real dependency-cycle risk, and still couples
the libraries to app-layer concerns transitively through the new crate.

**Risk Assessment**:

- **Technical Risk**: High. A crate sitting between the binaries and the
  libraries, depended on by both, is a standing cycle risk as the workspace
  evolves.
- **Schedule Risk**: Medium. Introducing and maintaining a tenth workspace
  member adds ongoing overhead.
- **Ecosystem Risk**: High. Libraries would still be transitively coupled to
  app-layer concerns through `mif-app`.

## Decision

We enforce a **strict one-directional dependency**: `mif-cli` and `mif-mcp`
depend on whichever of the seven library crates they call directly; none of
`mif-core`, `mif-schema`, `mif-ontology`, `mif-problem`, `mif-frontmatter`,
`mif-embed`, or `mif-store` ever depends on either binary crate.

## Consequences

### Positive

1. **Standalone libraries**: the seven libraries are genuinely standalone
   and reusable by a third party with zero interest in a CLI or MCP server.
2. **No leaked app-layer concerns**: no CLI/argument-parsing/MCP-schema
   concern ever leaks into a library's public API.

### Negative

1. **Accepted duplication**: `mif-cli` and `mif-mcp` carry some literal
   duplication of logic that cannot be shared without creating a
   library-external helper crate. For example, both binaries have their own
   local `warm_embedding_model_cache` test helper, documented inline in each
   binary's own test module as intentional: `cargo test` runs tests in
   parallel within one process, and every test that calls `Embedder::load()`
   races the others to download/lock the same model blob on a cold cache.
   Each binary's own `std::sync::Once`-guarded helper avoids that race
   independently. This duplication is accepted, not accidental.

### Neutral

1. Adding a third binary consumer in the future requires zero changes to
   any of the seven libraries.

## Decision Outcome

The decision achieves its primary objective — libraries that stay
independently reusable — measured by: none of `mif-core`, `mif-schema`,
`mif-ontology`, `mif-problem`, `mif-frontmatter`, `mif-embed`, or
`mif-store` ever lists `mif-cli` or `mif-mcp` as a dependency in its own
`Cargo.toml`.

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace, Not a Root Package](https://modeled-information-format.github.io/mif-rs/adr/0003-virtual-cargo-workspace/) —
  establishes the workspace structure this dependency direction governs.

## Links

- [Cargo Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html) - How Cargo resolves dependencies between workspace members
- [Package Layout](https://doc.rust-lang.org/cargo/guide/project-layout.html) - Cargo's convention for separating library (`src/lib.rs`) from binary (`src/bin/`) crates
- [Acyclic Dependencies Principle](https://en.wikipedia.org/wiki/Acyclic_dependencies_principle) - The general design principle this one-directional rule is an instance of

## More Information

- **Date**: 2026-07-03 (retroactively documents an established, ongoing
  architectural constraint).
- **Source**: this repository's crate dependency graph, as designed from
  workspace bootstrap onward.

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Verified none of the seven library crates' `Cargo.toml` files list `mif-cli` or `mif-mcp` as a dependency | `crates/{mif-core,mif-schema,mif-ontology,mif-problem,mif-frontmatter,mif-embed,mif-store}/Cargo.toml` | - | accepted |

**Summary:** Read each of the seven library crates' `Cargo.toml` files;
none reference either binary crate.

**Action Required:** None — this ADR documents current, already-adopted
practice.
