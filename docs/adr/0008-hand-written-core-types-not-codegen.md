---
title: "Hand-Written Core Types, Not Schema-to-Rust Codegen"
description: "Hand-write and field-verify mif-core's four public types directly against the live MIF JSON Schema, rather than generating them from mif.schema.json via a schema-to-Rust codegen tool like typify."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - codegen
  - mif-core
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
  - 0006-vendor-json-schema-at-compile-time.md
---

# ADR-0008: Hand-Written Core Types, Not Schema-to-Rust Codegen

## Status

Accepted

## Context

### Background and Problem Statement

`mif-core` defines four public types shared across the `mif-rs` workspace:
`OntologyReference`, `EntityReference`, `EntityData`, and `ConceptType`
(`crates/mif-core/src/ontology.rs`, `entity.rs`, `concept.rs`). Each is
hand-written and field-verified directly against the corresponding definition
in the canonical MIF JSON Schema (`mif.schema.json`,
`definitions/entity-reference.schema.json`), rather than generated from those
schema files via a schema-to-Rust codegen tool such as `typify`. We need to
record why this scoped, low-drift-risk surface is hand-maintained instead of
generated, and under what condition that would change.

### Current Limitations

1. **No prior decision record**: hand-writing these four types has been the
   approach since `mif-core`'s initial implementation, but the reasoning —
   and the codegen alternative it forecloses — has never been written down.
2. **Re-litigation risk**: without a citable record, a future contributor
   could reasonably propose introducing `typify` "to stay in sync with the
   schema automatically," not realizing the tradeoff was already considered.

## Decision Drivers

### Primary Decision Drivers

1. **Idiomatic fit over generic tooling**: `mif-core`'s types must expose
   this workspace's established conventions — consuming-self builders (see
   the "Builder Pattern" convention in this repository's `CLAUDE.md`) and the
   closed-enum-or-custom `EntityType` fallback
   (`#[serde(untagged)] enum EntityType { Known(KnownEntityType),
   Custom(String) }`, `crates/mif-core/src/entity.rs` lines 72-79) that
   preserves round-trip fidelity for schema values the closed variant
   doesn't cover — and generic codegen tooling does not naturally produce
   either pattern without heavy post-generation customization.
2. **Scale of the current surface**: four types, each with a small, stable
   field set, is small enough to hand-verify against the live schema at
   review time without meaningful drift risk at this stage of the project.

### Secondary Decision Drivers

1. **No added build-time dependency**: hand-writing avoids adding a codegen
   tool (and its own maintenance surface) to the build for a four-type
   surface that changes rarely.

## Considered Options

### Option 1: Generate the types from `mif.schema.json` via a tool like `typify`

**Description**: Run `typify` (or a comparable schema-to-Rust generator)
against `mif.schema.json` and its referenced definitions to produce
`mif-core`'s types automatically, regenerating on schema changes.

**Advantages**:

- Regeneration keeps the types mechanically in sync with `mif.schema.json`
  without a manual review step catching each field addition.
- Removes hand-transcription error for a schema that changes over time,
  since the generator reads the schema directly rather than a human
  re-deriving each field.

**Disadvantages**:

- `typify`'s generated output does not produce this workspace's consuming-self
  builder pattern or the `EntityType::Known(..) | Custom(String)` fallback;
  retrofitting both onto generated code requires substantial post-generation
  hand-editing.
- At this scale (four types), that hand-editing is likely to produce more
  total code and more ongoing maintenance surface than hand-writing the four
  types directly, for no real time savings.

**Risk Assessment**:

- **Technical Risk**: Medium. Generated output would need non-trivial,
  recurring hand-editing to match existing idioms, which is itself a source
  of bugs if a regeneration overwrites a hand-edit.
- **Schedule Risk**: Low. A four-type surface is small either way.
- **Ecosystem Risk**: Medium. Introduces a codegen tool dependency and a
  generate-then-edit workflow for a surface that doesn't yet need it.

### Option 2: Hand-write and field-verify the types directly against the live schema (chosen)

**Description**: Write `OntologyReference`, `EntityReference`, `EntityData`,
and `ConceptType` by hand in `mif-core`, verifying each field against the
corresponding schema definition at review time, and following this
workspace's established idioms (consuming-self builders, the
`Known(..) | Custom(String)` fallback) directly rather than retrofitting them
onto generated output.

**Advantages**:

- Produces an idiomatic Rust API — proper consuming-self builders, the
  `Known | Custom` fallback preserving unknown schema values verbatim —
  without fighting a codegen tool's own conventions or output shape.
- No codegen tool dependency, no generate-then-edit workflow, no risk of a
  regeneration step silently discarding hand-edits.

**Disadvantages**:

- A schema field addition to any of the four types requires a manual,
  reviewed code change rather than a regeneration step.

**Risk Assessment**:

- **Technical Risk**: Low. Four types, verified field-by-field against the
  live schema at review time; no generated-code idiom mismatch to manage.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low, at the current scale of the type surface.

### Option 3: Generate a first pass with `typify`, then hand-edit the output to retrofit the idioms

**Description**: Use `typify` to produce an initial draft of the types, then
manually edit the generated code to add the consuming-self builders and the
`Known(..) | Custom(String)` fallback this workspace requires.

**Advantages**:

- The initial draft removes some of the transcription work of starting from
  a blank file, since the generator produces a field-complete skeleton from
  the schema before any hand-editing begins.

**Disadvantages**:

- Produces an artifact that looks machine-generated but is not actually
  regenerable without losing the hand-edits — arguably worse than either a
  pure hand-written or pure generated approach.
- Invites a future contributor to "just regenerate it" from the schema and
  silently destroy the customizations that make the types idiomatic.

**Disqualifying Factor**: an artifact that looks regenerable but isn't is a
trap for future contributors (human or agent), not a genuine middle ground
between Options 1 and 2.

**Risk Assessment**:

- **Technical Risk**: High. A future regeneration silently discards
  hand-edits with no compiler error to catch it.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: High. Misleads contributors about whether the file is
  safe to regenerate.

## Decision

We hand-write and field-verify `mif-core`'s four public types
(`OntologyReference`, `EntityReference`, `EntityData`, `ConceptType`) directly
against the live MIF JSON Schema, rather than generating them via a
schema-to-Rust codegen tool like `typify`.

This decision is explicitly scoped to the current four-type surface and is
revisable: revisit codegen if/when a fuller document-type surface — a full
`Mif` struct mirroring every optional field of `mif.schema.json`, not just
the current four types — gets built. That is the point at which
hand-maintenance drift risk would start to outweigh codegen's ergonomic cost.

## Consequences

### Positive

1. **Idiomatic Rust API**: proper consuming-self builders (see
   `EntityReference::with_entity_type`/`with_name`/`with_role`,
   `crates/mif-core/src/entity.rs` lines 40-59, and
   `OntologyReference::with_version`/`with_uri`,
   `crates/mif-core/src/ontology.rs` lines 38-51) and the `Known | Custom`
   fallback preserving unknown schema values verbatim, without fighting a
   codegen tool's own conventions or output shape.

### Negative

1. **Manual sync required**: a schema field addition to any of the four
   types requires a manual, reviewed code change rather than a regeneration
   step, so drift between the schema and `mif-core`'s types is possible if
   that sync step is skipped or missed in review.

### Neutral

1. This decision is explicitly scoped to the current four-type surface and
   stated as revisable — it is not a permanent rejection of codegen as a
   strategy, only a judgment that it isn't worth it yet at this scale.

## Decision Outcome

The decision achieves its primary objective — an idiomatic, hand-verified
four-type surface with no codegen dependency — measured by: `mif-core`'s four
public types remain hand-written and match the live schema's corresponding
definitions field-for-field. Verified by reading
`crates/mif-core/src/entity.rs`: `EntityType` (lines 72-79) is
`#[serde(untagged)] enum EntityType { Known(KnownEntityType), Custom(String) }`,
with `KnownEntityType` (lines 82-94) enumerating the schema's closed set
(`Person`, `Organization`, `Technology`, `Concept`, `File`), and
`EntityReference`/`EntityData` following the consuming-self builder pattern
throughout.

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace](https://modeled-information-format.github.io/mif-rs/adr/0003-virtual-cargo-workspace/)
- [ADR-0006: Vendor the Canonical JSON Schema at Compile Time, Not Fetch at Validate Time](https://modeled-information-format.github.io/mif-rs/adr/0006-vendor-json-schema-at-compile-time/)

## Links

- [`typify`](https://github.com/oxidecomputer/typify) - the schema-to-Rust codegen tool considered and rejected in Option 1/Option 3
- [JSON Schema 2020-12 specification](https://json-schema.org/draft/2020-12) - the schema dialect `mif.schema.json` declares and that `typify` would read from
- [Serde: enum representations - untagged](https://serde.rs/enum-representations.html#untagged) - the mechanism behind `EntityType`'s `Known(..) | Custom(String)` fallback this ADR requires any generated code to reproduce

## More Information

- **Date**: 2026-07-03
- **Source**: `crates/mif-core/src/` (retroactively documents an established,
  ongoing design)

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| `EntityType::Known(KnownEntityType) \| Custom(String)` untagged fallback present as described | crates/mif-core/src/entity.rs | 72-94 | accepted |

**Summary:** Current implementation matches the decision as recorded — the
four `mif-core` types are hand-written, field-verified against the live
schema, and follow the workspace's consuming-self builder and
`Known | Custom` conventions.

**Action Required:** None — this ADR documents current, already-adopted
practice.
