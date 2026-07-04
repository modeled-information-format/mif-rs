---
title: "Per-Crate thiserror Error Enums, No Shared Top-Level Error Type"
description: "Give each mif-rs crate its own thiserror-derived error enum scoped to that crate's own failure modes, rather than a shared top-level error type, while every enum implements mif_problem::ToProblem for a uniform RFC 9457 envelope."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - error-handling
  - architecture
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
  - 0004-libraries-never-depend-on-binaries.md
---

# ADR-0005: Per-Crate thiserror Error Enums, No Shared Top-Level Error Type

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs`'s library crates each fail in genuinely different ways: `mif-schema`
fails on JSON Schema validation, `mif-ontology` fails on corpus/graph
resolution (cyclic `extends` chains, missing namespaces), `mif-frontmatter`
fails on markdown/JSON-LD projection, `mif-embed` fails on model loading, and
`mif-store` fails on SQLite operations. None of these failure spaces overlap
in any way that a single shared error type could represent without either
flattening every crate's variants into a lowest-common-denominator shape or
boxing them behind a dynamic `dyn Error`.

At the same time, every one of these failures must also render as a
consistent [RFC 9457] `application/problem+json` envelope, since `mif-cli`
and `mif-mcp` now answer to two audiences — the human reading the terminal
and the LLM agent parsing structured output to decide whether to retry,
escalate, or abandon. That envelope shape needs to be uniform across all
seven error-producing crates in the workspace (`mif-schema`, `mif-ontology`,
`mif-frontmatter`, `mif-embed`, `mif-store`, `mif-cli`, `mif-mcp`) without
forcing them into one shared error hierarchy to get there.

### Current Limitations

1. **No prior workspace-wide error convention**: absent a decision, each new
   crate could independently choose `thiserror`, `anyhow`, `eyre`, or ad hoc
   `String` errors, leaving callers with no consistent way to match on
   failure kinds across crates.
2. **The RFC 9457 envelope needs one shared shape, not one shared type**: the
   agent-facing `application/problem+json` output (`type`, `title`, `status`,
   `detail`, `instance`, plus the `retry_after`/`suggested_fix`/`code_actions`
   extensions) must look identical regardless of which crate raised the
   error, but the underlying Rust error values are not interchangeable.
3. **Matchability matters to callers**: a caller distinguishing a
   schema-validation failure from a store I/O failure needs variant-level
   pattern matching, which a fully erased/dynamic error type would foreclose.

## Decision Drivers

### Primary Decision Drivers

1. **Scoped failure modes**: each crate's error enum shall stay scoped to
   that crate's own actual failure modes, not a shared vocabulary that fits
   no single crate well.
2. **Zero-overhead derivation**: error `Display`/`std::error::Error`
   implementation shall not add runtime overhead or require hand-written
   boilerplate per variant.
3. **Uniform machine-readable envelope**: every crate's errors shall render
   as the same RFC 9457 `application/problem+json` shape for agent
   consumers, without requiring the crates to share an error type to get
   there.

### Secondary Decision Drivers

1. **Variant-level matchability**: callers (human code or an agent) shall be
   able to match on a specific error variant (e.g. a cyclic-ontology error)
   rather than only a generic failure.

## Considered Options

### Option 1: One shared top-level `mif-rs::Error` enum

**Description**: Define a single top-level `Error` enum that every crate's
fallible functions return or convert into.

**Advantages**:

- One error type to learn across the whole workspace.

**Disadvantages**:

- Forces every library crate to either depend on a shared error crate — real
  coupling across the entire dependency graph, in a workspace that otherwise
  keeps crates independently scoped — or box/erase into `anyhow`-style
  dynamic errors, losing variant-level matchability for callers who need to
  distinguish, say, a schema-validation failure from a store I/O failure.
- A shared enum's variants inevitably become either too generic to be useful
  per-crate, or bloated with every crate's specific failure modes mixed
  together.

**Risk Assessment**:

- **Technical Risk**: Medium. A shared error type becomes a coupling point
  every crate must agree on before it can ship a new failure mode.
- **Schedule Risk**: Medium. Adding a new crate to the workspace means
  negotiating changes to a shared type other crates also depend on.
- **Ecosystem Risk**: High. Undermines the workspace's existing pattern of
  independently scoped library crates (see ADR-0004).

### Option 2: Each crate owns its own `thiserror`-derived enum (chosen)

**Description**: Each crate defines its own `thiserror`-derived error enum
(`MifSchemaError`, `OntologyError`, `FrontmatterError`, `EmbedError`,
`StoreError`, plus the two binaries' own `CliError`/`McpError`), and every one
of these enums implements the shared `mif_problem::ToProblem` trait for the
RFC 9457 envelope, using `mif_problem::ProblemMeta` (slug, version, title,
status, exit code) to keep each crate's own URI/status/exit-code bookkeeping
in one place.

**Advantages**:

- Each error enum stays scoped to that crate's own actual failure modes —
  `MifSchemaError` describes schema-validation failures, `OntologyError`
  describes corpus/graph-resolution failures, and so on, with no forced
  overlap.
- `thiserror` gives zero-runtime-overhead `Display`/`Error` derivation from
  attributes, with no hand-written boilerplate per variant.
- `mif_problem::ToProblem` gives every crate the same RFC 9457 envelope shape
  without requiring them to share an error type — the envelope is uniform,
  the underlying error values are not.
- Callers can exhaustively match a specific crate's own variants.

**Disadvantages**:

- `mif-cli`'s and `mif-mcp`'s own error enums must explicitly wrap each
  library error type via `#[from]` variants rather than getting a shared
  conversion for free from a common base error type.

**Risk Assessment**:

- **Technical Risk**: Low. `thiserror` is already the workspace's existing
  error-derivation convention (see this repo's `CLAUDE.md`, "Why `thiserror`
  for Errors").
- **Schedule Risk**: Low. Adding a new crate's error enum is independent work
  that does not require negotiating a shared type.
- **Ecosystem Risk**: Low. Matches the workspace's existing pattern of
  independently scoped library crates.

### Option 3: `anyhow`/`eyre` dynamic errors everywhere, no typed enums

**Description**: Every fallible function returns `anyhow::Error` (or `eyre`'s
equivalent) instead of a typed enum, with no per-variant structure at all.

**Advantages**:

- Fastest option to adopt — no per-variant enum to define, no
  `mif_problem::ProblemMeta` bookkeeping to author per failure mode.
- No enum to keep exhaustively up to date as new failure modes appear;
  `anyhow::Error`/`eyre::Report` accept any `std::error::Error` without a
  matching arm.

**Disadvantages**: Loses the ability for a caller — or an agent parsing
structured output — to match on a specific error variant, which undermines
the entire reason `mif-problem` exists: a structured, machine-readable error
surface. A dynamic error's message is exactly what an agent cannot reliably
branch on across a stable, versioned `type` URI.

**Disqualifying Factor**: erasing every error to a dynamic type is
incompatible with the workspace's core requirement of a stable, matchable,
per-variant RFC 9457 envelope.

**Risk Assessment**:

- **Technical Risk**: Low to adopt, but forecloses the workspace's actual
  requirement.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: High. Defeats the purpose of `mif-problem` existing at
  all.

## Decision

We give **each `mif-rs` crate its own `thiserror`-derived error enum**,
scoped to that crate's own failure modes, with **no shared top-level error
type**. Every error-producing crate's enum implements
`mif_problem::ToProblem` directly for a uniform RFC 9457 envelope:

- `mif-schema` → `MifSchemaError`
- `mif-ontology` → `OntologyError`
- `mif-frontmatter` → `FrontmatterError`
- `mif-embed` → `EmbedError`
- `mif-store` → `StoreError`
- `mif-cli` → `CliError`
- `mif-mcp` → `McpError`

Each enum defines its own per-variant `mif_problem::ProblemMeta` (slug,
version, title, status, exit code) and converts it to a full
`ProblemDetails` envelope via `ProblemMeta::into_details`. `mif-cli` and
`mif-mcp` wrap each of the five library error types as `#[from]` variants on
their own `CliError`/`McpError` enums.

## Consequences

### Positive

1. **Variant-level matchability**: callers can exhaustively match each
   crate's own error variants, e.g. `matches!(err, OntologyError::Cycle(_))`
   to detect a cyclic `extends` chain specifically, rather than only a
   generic failure.
2. **Uniform envelope without a shared hierarchy**: `mif_problem::ToProblem`
   gives every crate the same RFC 9457 shape for agent consumers without
   forcing a shared inheritance/coupling relationship across crates.
3. **Localized extension**: adding a new failure mode to one crate is a
   one-arm addition to that crate's own `meta()`/`to_problem()` match, not a
   change rippling through a type other crates also depend on.

### Negative

1. **Explicit wrapping in the binaries**: `mif-cli`'s and `mif-mcp`'s own
   error enums must explicitly wrap each of the five library error types via
   `#[from]` variants, rather than getting a shared conversion for free from
   a common base error type. This is a fixed, small, enumerable cost (one
   `#[from]` variant per library crate) paid once per binary.

### Neutral

1. Adding a new library crate to the workspace means adding a new,
   independent error enum for it, not extending a shared one. This workspace
   deliberately has no shared top-level error type by design — stated
   explicitly in `mif-problem`'s own module doc comment.
2. `mif-problem` itself supplies only the envelope shape
   (`ProblemDetails`/`ProblemMeta`/`ToProblem`), not a base error type — it is
   not a candidate for becoming the shared error type this decision rejects.

## Decision Outcome

The decision achieves its primary objective — scoped, matchable error enums
sharing one uniform RFC 9457 envelope with no shared top-level error type —
measured by: every one of `mif-schema`, `mif-ontology`, `mif-frontmatter`,
`mif-embed`, `mif-store`, `mif-cli`, and `mif-mcp`'s error enums implements
`mif_problem::ToProblem` directly, and none of them depend on a shared
top-level error type crate other than `mif-problem` itself (which supplies
only the envelope shape, not a base error type). `mif-problem`'s own module
doc comment states this design intent explicitly:

> "This workspace deliberately has no shared top-level error type ... —
> `mif-schema`, `mif-ontology`, `mif-frontmatter`, `mif-embed`, and
> `mif-store` each fail in genuinely different ways and keep their own
> `thiserror` enum. This crate does not change that: instead of one central
> `Error` enum with a `meta()` match ..., each crate's own error enum
> implements `ToProblem` directly, using `ProblemMeta` to keep its own
> per-variant type-URI/status/exit-code bookkeeping in one place."

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace](0003-virtual-cargo-workspace.md) —
  establishes the multi-crate workspace structure whose members each own an
  independent error enum under this decision.
- [ADR-0004: Libraries Never Depend on Binaries](0004-libraries-never-depend-on-binaries.md) —
  the same directional-dependency discipline that keeps library crates from
  depending on binary crates also keeps them from being forced into a shared
  error type only the binaries would otherwise motivate.

## Links

- [RFC 9457: Problem Details for HTTP APIs](https://www.rfc-editor.org/rfc/rfc9457)

## More Information

- **Date**: 2026-07-03 (retroactively documents an established, ongoing
  architectural pattern)
- **Source**: `crates/mif-problem/src/lib.rs` and its consuming crates
  (`crates/mif-schema/src/lib.rs`, `crates/mif-ontology/src/lib.rs`,
  `crates/mif-frontmatter/src/lib.rs`, `crates/mif-embed/src/lib.rs`,
  `crates/mif-store/src/lib.rs`, `crates/mif-cli/src/main.rs`,
  `crates/mif-mcp/src/main.rs`)

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Module doc comment states "no shared top-level error type" as explicit design intent; every consuming crate's error enum implements `ToProblem` directly | crates/mif-problem/src/lib.rs | 13-22 | accepted |

**Summary:** Decision matches current, already-implemented practice across
all seven error-producing crates; no prior shared error type exists to
migrate away from.

**Action Required:** None — this ADR documents current, already-adopted
practice.
