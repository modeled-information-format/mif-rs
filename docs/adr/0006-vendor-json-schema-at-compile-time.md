---
title: "Vendor the Canonical JSON Schema at Compile Time, Not Fetch at Validate Time"
description: "Embed the canonical MIF JSON Schema files in mif-schema via include_str! at compile time and resolve every $ref through a custom offline jsonschema::Registry, instead of fetching schemas over HTTP at validate() time."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - schema
  - validation
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

# ADR-0006: Vendor the Canonical JSON Schema at Compile Time, Not Fetch at Validate Time

## Status

Accepted

## Context

### Background and Problem Statement

`mif-schema` validates MIF documents, standalone citation objects, and
ontology definitions against the canonical MIF JSON Schema
(`mif.schema.json`, `citation.schema.json`, `ontology.schema.json`,
`definitions/entity-reference.schema.json`), synced from the canonical MIF
repository's own `schema/` directory. These schema files are embedded
directly into the compiled crate via `include_str!` and resolved entirely
offline through a custom `jsonschema::Registry`, with `jsonschema`'s default
HTTP/file-resolver features explicitly disabled. We need to record why
validation is wired this way instead of fetching the canonical schema over
the network at validate time.

### Current Limitations

1. **No prior decision record**: this vendoring approach has been in place
   since `mif-schema`'s initial implementation, but the reasoning behind it —
   and the alternatives it forecloses — has never been written down.
2. **Re-litigation risk**: without a citable record, a future contributor
   could reasonably propose fetching schemas live "to always be current,"
   not realizing that was already considered and rejected.

## Decision Drivers

### Primary Decision Drivers

1. **Determinism and reproducibility**: validating the same document against
   the same crate version shall always produce the same result, independent
   of network conditions or the remote content at the moment of the call.
2. **Offline and airgapped operation**: validation shall work with no
   network access at all, including inside sandboxed or airgapped CI.

### Secondary Decision Drivers

1. **Minimal dependency footprint**: the crate's own dependency surface
   should not carry an HTTP client stack it does not otherwise need for
   anything.

## Considered Options

### Option 1: Fetch the canonical schemas over HTTP at first validate() call

**Description**: Fetch the canonical schemas over HTTP from the MIF spec
repository or `mif-spec.dev` at first `validate()` call, caching the result
in memory for the process lifetime.

**Advantages**:

- Schema updates on `mif-spec.dev` become visible to running processes
  without a `mif-schema` release, since there is no vendored copy to fall
  behind the canonical source.
- No manual re-vendoring step to remember or forget — the schema files
  never need to be copied into this repository at all.

**Disadvantages**:

- Non-deterministic: the result depends on network availability and the
  remote content at the moment of the call, not just on the crate version.
- Breaks entirely in offline or sandboxed CI.
- Requires pulling in `jsonschema`'s default HTTP-resolver feature set,
  which itself pulls in a full `reqwest`/`rustls`/`aws-lc-rs` stack this
  crate does not otherwise need for anything — every `$ref` in this
  workspace resolves offline via the custom `Registry`.

**Risk Assessment**:

- **Technical Risk**: High. Validation correctness becomes dependent on
  network state.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: High. Fails outright in airgapped or sandboxed CI, the
  exact environment this workspace's own release pipeline runs in.

### Option 2: Embed the schemas at compile time, resolve offline (chosen)

**Description**: Embed the schema JSON files via `include_str!` at compile
time; resolve every `$ref` through a custom, offline `jsonschema::Registry`;
pin `jsonschema`'s `default-features = false` in `[workspace.dependencies]`
specifically to avoid the HTTP-resolver stack.

**Advantages**:

- Validation is fully offline and deterministic, identical on a developer
  machine and in sandboxed CI.
- No HTTP client dependency surface at all for the core validation path.

**Disadvantages**:

- Vendored copies can drift from the canonical MIF repository's own
  `schema/` directory if the manual re-vendoring step is skipped or
  forgotten.

**Risk Assessment**:

- **Technical Risk**: Low. Plain `include_str!` and an offline registry, no
  new infrastructure.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

### Option 3: Fetch the schemas once at build.rs time, bake into the binary

**Description**: Fetch the canonical schemas from the network during
`build.rs` execution and embed the fetched result into the compiled binary.

**Advantages**:

- The compiled binary is still fully offline and deterministic at runtime,
  same as Option 2, since the network access happens only once at build
  time.
- Removes the manual re-vendoring step: each build automatically pulls the
  current canonical schema instead of relying on someone to copy files in.

**Disadvantages**:

- Still requires network access at build time, which breaks fully airgapped
  builds and complicates build reproducibility across machines and CI
  runners.
- Does not solve Option 1's underlying dependency-surface problem, since the
  resolution mechanism at runtime would still need to be offline-capable for
  any `$ref` within the fetched documents.

**Risk Assessment**:

- **Technical Risk**: Medium. Build-time network dependency is a different
  failure mode than runtime, but still a failure mode.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Medium. Airgapped build environments still break.

## Decision

We vendor the canonical MIF JSON Schema files into `mif-schema` at compile
time via `include_str!`, and resolve every `$ref` entirely offline through a
custom `jsonschema::Registry`.

Concretely (`crates/mif-schema/src/lib.rs`):

- `MIF_SCHEMA`, `CITATION_SCHEMA`, `ONTOLOGY_SCHEMA`, and
  `ENTITY_REFERENCE_SCHEMA` are each embedded via `include_str!` from
  `src/schemas/`, synced from the canonical MIF repository's `schema/`
  directory.
- `build_registry` constructs a `jsonschema::Registry` seeded with the
  entity-reference schema at its canonical `$id`
  (`https://mif-spec.dev/schema/definitions/entity-reference.schema.json`)
  and calls `.prepare()` so `$ref` resolution never touches the network.
- The workspace root `Cargo.toml` pins
  `jsonschema = { version = "0.46.9", default-features = false }` in
  `[workspace.dependencies]`, specifically to exclude `jsonschema`'s default
  HTTP-resolver feature set.
- `mif-schema`'s own `[dependencies]` (`mif-core`, `mif-problem`,
  `jsonschema`, `serde_json`, `thiserror`) carry no HTTP client crate.

## Consequences

### Positive

1. **Deterministic, fully offline validation**: `validate_document`,
   `validate_citation`, and `validate_ontology_definition` run identically
   on a developer machine and in sandboxed CI, with zero network access.
2. **No HTTP dependency surface**: the crate's core validation path carries
   no HTTP client stack at all.

### Negative

1. **Vendored-copy drift risk**: the vendored schemas can fall out of sync
   with the canonical MIF repository's `schema/` directory if the
   re-vendoring step is skipped; this is a deliberate, documented manual
   step, not an automatic one.

### Neutral

1. A schema version bump requires a code change — re-vendor the files, bump
   the version, release — rather than a runtime configuration change or a
   live document fetch.

## Decision Outcome

The decision achieves its primary objective — fully offline, deterministic
validation with no HTTP dependency surface — measured by: `mif-schema`'s
`Cargo.toml` `[dependencies]` carries zero HTTP client crate (verified:
`mif-core`, `mif-problem`, `jsonschema`, `serde_json`, `thiserror` only), the
workspace root `Cargo.toml` pins `jsonschema`'s `default-features = false`,
and `validate_document`/`validate_citation`/`validate_ontology_definition`
succeed with zero network access.

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace](0003-virtual-cargo-workspace.md)

## Links

- [JSON Schema 2020-12 Specification](https://json-schema.org/draft/2020-12/schema) - the draft this crate's vendored schemas and `jsonschema` crate target
- [`jsonschema` crate documentation](https://docs.rs/jsonschema/latest/jsonschema/) - the validator crate whose `Registry` type resolves `$ref`s offline in `build_registry`
- [`include_str!` macro reference](https://doc.rust-lang.org/std/macro.include_str.html) - the compile-time embedding mechanism this decision relies on
- [MIF canonical schema source (`schema/`)](https://github.com/modeled-information-format/MIF/tree/main/schema) - the upstream directory these vendored copies are synced from

## More Information

- **Date**: 2026-07-03
- **Source**: `crates/mif-schema/src/lib.rs` and
  `crates/mif-schema/src/schemas/` (retroactively documents an established,
  ongoing design)

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Schemas embedded via `include_str!`; offline `Registry` constructed via `build_registry`/`.prepare()` | crates/mif-schema/src/lib.rs | 17-21, 125-133 | accepted |

**Summary:** Current implementation matches the decision as recorded — no
network access occurs at schema-compilation or validation time.

**Action Required:** None — this ADR documents current, already-adopted
practice.
