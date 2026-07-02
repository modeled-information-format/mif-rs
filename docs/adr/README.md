---
id: how-to-manage-mif-rs-adr-lifecycle
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/process
title: How to Propose, Accept, Supersede, or Deprecate an ADR
tags:
  - how-to
  - process
  - adr
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
relationships:
  - type: relates-to
    target: 0001-use-architectural-decision-records.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Manage the mif-rs ADR Lifecycle
  entity_type: how-to-guide
---

# How to Propose, Accept, Supersede, or Deprecate an ADR

Manage an Architectural Decision Record's lifecycle in `mif-rs`, from first
draft through to acceptance or eventual supersession.

## Prerequisites

- A change with real, considered alternatives — not a task or a requirement
  (see [ADR-0001](0001-use-architectural-decision-records.md)).
- Familiarity with the Structured MADR format this repository uses (via the
  `mif-docs:adr` skill), not plain MADR.

## Propose a new ADR

1. Create a new file in `docs/adr/` named `NNNN-title-with-dashes.md`, using
   the next sequential number.
2. Author it with the `mif-docs:adr` skill so it carries the required
   Structured MADR sections and MIF frontmatter.
3. Set `status: proposed` in the frontmatter.
4. Open a pull request.

## Accept an ADR

1. After discussion and approval on the pull request, change `status` to
   `accepted`.
2. Merge the pull request.

## Supersede an ADR

1. Create a new ADR (per "Propose a new ADR" above) that documents the
   replacement decision.
2. In the old ADR's frontmatter, set `status: superseded` and add a
   `relationships` entry of type `superseded-by` pointing at the new ADR.
3. Set the new ADR's `status` to `accepted` once approved.

## Deprecate an ADR

1. If a decision is no longer relevant but has no direct replacement, set
   `status: deprecated` in its frontmatter.
2. Add a note in the ADR's `## Audit` section explaining why.

An accepted ADR's outcome is never edited in place — a changed decision gets
a new, superseding ADR instead.

## ADR Index

- [ADR-0001](0001-use-architectural-decision-records.md) — Use Architectural Decision Records
- [ADR-0002](0002-documentation-directory-structure.md) — Documentation Directory Structure
