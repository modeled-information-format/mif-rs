---
title: "Generic Frontmatter Pass-Through, Not a Curated Field List"
description: "mif-frontmatter passes every frontmatter/JSON-LD key through generically on the markdown <-> JSON-LD round trip, deliberately deviating from mif_convert.py (the canonical Python reference converter)'s fixed passthrough-field-list behavior."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - frontmatter
  - round-trip
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

# ADR-0007: Generic Frontmatter Pass-Through, Not a Curated Field List

## Status

Accepted

## Context

### Background and Problem Statement

`mif-frontmatter` ports the canonical `mif_convert.py` reference converter
(from the `MIF` spec repository) to Rust, projecting a concept file's YAML
frontmatter to and from its derived JSON-LD form. `mif_convert.py`'s
`jsonld_to_md` only recovers a fixed list of passthrough fields from a
JSON-LD document on the reverse leg of that projection, silently dropping any
other frontmatter key on the full `markdown -> json-ld -> markdown` pipeline
— even though `serialize_markdown` alone would have preserved it.

Real corpora hit this. `research-harness-template`'s own findings and Level-3
report documents carry fields — `slug`, `version`, harness-specific
`extensions.harness` data — that the fixed Python passthrough list never
anticipated. This crate's own `roundtrip_lossless` previously failed with
`RoundTripDrift` against those real documents until it stopped curating a
fixed field list of its own.

### Current Limitations

1. **Silent data loss on round trip**: any frontmatter key outside
   `mif_convert.py`'s fixed passthrough list survives `parse_markdown` and
   `md_to_jsonld`, but is dropped by `jsonld_to_md`, so a full
   `markdown -> json-ld -> markdown` cycle is lossy for exactly the
   documents that extend frontmatter beyond the reference converter's own
   test cases.
2. **Reference converter's own bug, not a spec constraint**: the module's
   doc comment records that `mif.schema.json`'s root object schema does not
   set `additionalProperties: false`, so unrecognized top-level frontmatter
   keys are already spec-legal — dropping them silently on round trip is a
   correctness bug in `mif_convert.py`, not a behavior worth preserving for
   compatibility's sake.
3. **Real regression, not a hypothetical**: `research-harness-template`'s
   Level-3 report frontmatter already embeds `@context`/`@type`/`@id`/
   `conceptType` directly, plus a `slug`/`version` pair the v1.0 canonical
   shape doesn't define — a real, in-the-wild document this crate's own
   `roundtrip_lossless` proof must hold against, not just synthetic
   fixtures shaped like the reference converter's own test cases.

## Decision Drivers

### Primary Decision Drivers

1. **The schema already permits it**: `mif.schema.json`'s root object schema
   does not set `additionalProperties: false`, so silently dropping
   unrecognized top-level keys contradicts the spec this crate is meant to
   conform to.
2. **`roundtrip_lossless` must hold against real documents**: the crate's
   lossless round-trip proof is only meaningful if it holds against real,
   in-the-wild MIF documents (like `research-harness-template`'s reports),
   not just synthetic fixtures shaped like the reference converter's own
   test cases.

### Secondary Decision Drivers

1. **Parity with a reference implementation is not an end in itself**:
   matching `mif_convert.py`'s behavior is valuable where it reflects a
   deliberate design choice, but not where it reflects an unexamined bug.

## Considered Options

### Option 1: Replicate `mif_convert.py`'s fixed passthrough list exactly

**Description**: Port `mif_convert.py`'s fixed passthrough-field list
verbatim into `jsonld_to_md`, for behavioral parity with the reference
implementation. This was effectively the status quo this crate started from.

**Advantages**:

- Byte-for-byte behavioral parity with the canonical Python reference
  converter.

**Disadvantages**:

- Previously failed `roundtrip_lossless` with `RoundTripDrift` against real
  `research-harness-template` documents.
- Parity with a reference implementation's own bug is not a virtue, and the
  schema itself already permits the fields being dropped.

**Risk Assessment**:

- **Technical Risk**: High. Known to fail against real documents already in
  this ecosystem.
- **Schedule Risk**: Low. No new work beyond porting the existing list.
- **Ecosystem Risk**: High. Silently corrupts frontmatter for any downstream
  consumer whose documents extend beyond the reference converter's own field
  list.

### Option 2: Generic pass-through of every frontmatter/JSON-LD key (chosen)

**Description**: `md_to_jsonld` and `jsonld_to_md` pass every
frontmatter/JSON-LD key through generically; `FRONTMATTER_ORDER` governs
serialization order only, not which keys survive the round trip.

**Advantages**:

- Proven against real documents (`research-harness-template`'s own findings
  and Level-3 reports) that previously failed to round-trip losslessly under
  the old, curated-field-list behavior.
- Matches `mif.schema.json`'s own stated openness (no
  `additionalProperties: false` at the root).

**Disadvantages**:

- Surfaced a genuinely ambiguous identity case (see Consequences below):
  once every key round-trips generically, a document's `@id`/`conceptType`
  identity can arrive either via the v1.0 `id`/`type` shorthand or via
  already-projected literal `@context`/`@type`/`@id`/`conceptType` keys, and
  the two are indistinguishable from the JSON-LD value alone — this required
  introducing a separate `FrontmatterShape` enum to resolve.

**Risk Assessment**:

- **Technical Risk**: Low. Verified by this crate's own test suite, including
  a regression fixture for the real failure this fixes.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

### Option 3: Make the passthrough field list configurable per-caller

**Description**: Add an opt-in allowlist or denylist parameter to
`jsonld_to_md`, letting each caller decide which fields pass through.

**Advantages**:

- Would let a caller with an unusual compatibility need (e.g. deliberately
  reproducing `mif_convert.py`'s exact field list) opt into that behavior
  without forking the crate.

**Disadvantages**: Adds real API surface for no real benefit over simply
always passing everything through by default, and a caller could still
silently misconfigure it into `mif_convert.py`'s lossy behavior.

**Disqualifying Factor**: a configurable footgun that reproduces the exact
bug this decision exists to fix is worse than no configuration at all.

**Risk Assessment**:

- **Technical Risk**: Medium. Correct behavior depends on every caller
  configuring the option correctly.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Medium. A misconfigured caller silently reintroduces
  the data-loss bug this decision rejects.

## Decision

We adopt **Option 2: generic pass-through of every frontmatter/JSON-LD key**.
`md_to_jsonld` and `jsonld_to_md` pass every frontmatter/JSON-LD key through
generically; `FRONTMATTER_ORDER` governs serialization order only, never
which keys survive the round trip. This is a deliberate, documented
deviation from `mif_convert.py`'s behavior, recorded in the crate's own
module-level "Known deviations from the Python reference" doc comment.

## Consequences

### Positive

1. **Proven against real documents**: `research-harness-template`'s own
   findings and Level-3 reports, which previously failed to round-trip
   losslessly under the old curated-field-list behavior, now round-trip
   correctly.
2. **Matches the schema's own stated openness**: consistent with
   `mif.schema.json`'s root object schema, which does not set
   `additionalProperties: false`.

### Negative

1. **Surfaced a genuinely ambiguous identity case**: a document's
   `@id`/`conceptType` identity can be expressed either via the MIF v1.0
   `id`/`type` shorthand (which projects to `@id: urn:mif:{id}`) or via
   already-projected literal `@context`/`@type`/`@id`/`conceptType`
   frontmatter keys (e.g. `research-harness-template`'s own report
   documents) — both produce an identical `@id` string in the projected
   JSON-LD, so there is no way to distinguish them from the JSON-LD value
   alone on the reverse (`jsonld_to_md`) trip. This is resolved by the
   separate `FrontmatterShape` enum (`V1Canonical` | `PreProjected`), which
   `md_to_jsonld` auto-detects from the frontmatter it's given (a literal
   `@id` key present means `PreProjected`) but which `jsonld_to_md`'s caller
   must state explicitly, since it has no frontmatter to inspect on that
   direction.

### Neutral

1. This is a deliberate, documented deviation from the Python reference
   implementation's behavior, not an accidental compatibility regression —
   the module's own doc comment states this explicitly under "Known
   deviations from the Python reference."

## Decision Outcome

The decision achieves its primary objective — a lossless round trip against
real MIF documents, not just synthetic fixtures — measured by: the crate's
own test suite includes a regression fixture,
`the_real_rht_report_with_context_shaped_frontmatter_round_trips` (in
`crates/mif-frontmatter/src/lib.rs`'s test module), covering the
`research-harness-template` Level-3 report shape — already-JSON-LD-shaped
frontmatter (`@context`/`@type`/`@id`/`conceptType` as literal keys) plus a
`slug`/`version` pair the v1.0 canonical shape doesn't define — and this
fixture round-trips losslessly.

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace](https://modeled-information-format.github.io/mif-rs/adr/0003-virtual-cargo-workspace/) — establishes the workspace `mif-frontmatter` is a member of.

## Links

- [MIF Specification](https://mif-spec.dev) — the normative spec this crate conforms to.
- [`mif.schema.json`](https://mif-spec.dev/schema/mif.schema.json) — the canonical schema; its root object does not set `additionalProperties: false`, the decision driver behind this ADR.
- [`mif_convert.py`](https://github.com/modeled-information-format/MIF/blob/main/scripts/mif_convert.py) — the canonical Python reference converter this crate ports and deliberately deviates from.
- [JSON-LD 1.1 Specification](https://www.w3.org/TR/json-ld11/) — the JSON-LD data model `md_to_jsonld`/`jsonld_to_md` project frontmatter to and from.

## More Information

- **Date**: 2026-07-03 (retroactively documents an established, ongoing
  design).
- **Source**: `crates/mif-frontmatter/src/lib.rs`'s own "Known deviations
  from the Python reference" module doc comment.

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Regression test `the_real_rht_report_with_context_shaped_frontmatter_round_trips` proves the RHT Level-3 report shape (literal `@context`/`@type`/`@id`/`conceptType` keys plus `slug`/`version`) round-trips losslessly under generic pass-through | crates/mif-frontmatter/src/lib.rs | 1030-1061 | accepted |

**Summary:** Generic frontmatter/JSON-LD pass-through holds today, proven by
a real-document regression fixture rather than a synthetic one.

**Action Required:** None — this ADR documents current, already-implemented
behavior.
