---
title: "Documentation Directory Structure"
description: "Organize mif-rs documentation as a single flat docs/ tree with topic subdirectories (runbooks, security, distribution, testing, ux, workflows), rather than the upstream template's two-tier onboarding-guide plus reference split, since this workspace has no guided template-adoption flow."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: process
tags:
  - adr
  - documentation
  - process
status: accepted
created: 2026-07-02
updated: 2026-07-02
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0001-use-architectural-decision-records.md
---

# ADR-0002: Documentation Directory Structure

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs` was forked from `attested-delivery/rust-template`, whose docs/
directory was organized into two audience tiers: `docs/template/` (a guided
path from "use this template" to a first green CI run — `GETTING-STARTED.md`,
`CONFIGURATION.md`, `CI-WORKFLOWS.md`, `CUSTOMIZATION.md`, etc.) and
`docs/runbooks/` (operational procedures for maintainers), plus several
topic-specific reference subdirectories (`docs/workflows/`, `docs/security/`,
`docs/distribution/`, `docs/testing/`, `docs/ux/`, `docs/observability/`).

During this workspace's bootstrap, `docs/template/` and `docs/observability/`
were deleted along with a handful of auxiliary workflows they documented
(benchmark tracking, mutation testing, fuzz testing, a full Astro docs site)
that were judged premature for a v0.1.0 library workspace. `mif-rs` is not
itself a template someone else instantiates — it is the concrete, permanent
repository — so a "getting started with this template" onboarding guide has
no real audience here. We need a documentation structure that reflects that.

### Current Limitations

1. **No template-adoption audience to serve**: `docs/template/`'s guided path
   from "use this template" to first CI pass describes a workflow that does
   not apply to `mif-rs` itself.
2. **Orphaned observability docs**: `docs/observability/METRICS-DASHBOARD.md`
   documented a benchmark-regression tracking workflow that was removed as
   out of scope for this bootstrap.
3. **Two-tier split adds indirection without a second audience**: with the
   onboarding tier gone, maintaining a distinct top-level tier for "guides"
   versus "reference material" no longer reflects a real split in readership.

## Decision Drivers

### Primary Decision Drivers

1. **Match structure to actual audience**: the directory structure shall
   reflect the readers this repository actually has (contributors and
   maintainers), not a template-adoption audience it does not have.
2. **No orphaned documentation**: every file under `docs/` shall describe
   something that currently exists in this repository.

### Secondary Decision Drivers

1. **Low restructuring cost**: the existing topic subdirectories
   (`docs/workflows/`, `docs/security/`, `docs/distribution/`, `docs/testing/`,
   `docs/ux/`) are already well-organized reference material and should be
   kept rather than reorganized again.

## Considered Options

### Option 1: Keep the upstream two-tier structure as-is

**Description**: Retain `docs/template/` and `docs/observability/` unchanged,
even though they describe a template-adoption flow and a workflow that no
longer exist in this repository.

**Advantages**: No restructuring work at all — the directory tree stays
exactly as inherited from the upstream template.

**Disadvantages**: Documents content that is factually wrong for this repo —
`docs/template/GETTING-STARTED.md` would describe using a template that has
already been used, and `docs/observability/METRICS-DASHBOARD.md` would
describe a workflow that was deleted.

**Disqualifying Factor**: stale, factually incorrect documentation is worse
than no documentation on the same topic — it actively misleads.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: None.
- **Ecosystem Risk**: High. Misleads contributors and agents alike.

### Option 2: Single flat `docs/` tree with topic subdirectories (chosen)

**Description**: Drop the onboarding tier entirely; keep the existing
topic-organized reference subdirectories (`docs/runbooks/`, `docs/security/`,
`docs/distribution/`, `docs/testing/`, `docs/ux/`, `docs/workflows/`) plus
`docs/adr/`, with `docs/README.md` as a single index and `docs/DEPLOYMENT.md`
at the top level.

**Advantages**:

- Every remaining file describes something that actually exists in this repo.
- No indirection between an onboarding tier and a reference tier that no
  longer has two distinct audiences.
- Minimal restructuring: the topic subdirectories were already well-organized
  and are kept unchanged.

**Disadvantages**: Loses the upstream template's guided onboarding path —
if `mif-rs` ever needs to explain "how to bootstrap a project like this one"
again, that guide has to be written from scratch rather than adapted from
`docs/template/`.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

## Decision

We organize `mif-rs` documentation as a **single flat `docs/` tree with topic
subdirectories**, dropping the upstream template's separate onboarding tier.

Current structure:

- `docs/adr/` — architectural decision records (this document's own home).
- `docs/runbooks/` — operational procedures: `RELEASING.md`,
  `DEPENDENCY-UPDATES.md`, `SECURITY-RESPONSE.md`, `CI-TROUBLESHOOTING.md`.
- `docs/security/` — `ATTESTED-DELIVERY.md`, `SIGNED-RELEASES.md`.
- `docs/distribution/` — `ALTERNATIVE-REGISTRIES.md`, `DOCKER-REGISTRIES.md`,
  `PACKAGE-MANAGERS.md`.
- `docs/testing/` — `PROPERTY-BASED-TESTING.md`.
- `docs/ux/` — `MAN-PAGES.md`, `SHELL-COMPLETIONS.md`.
- `docs/workflows/` — one reference page per CI/CD workflow this repo
  actually runs.
- `docs/DEPLOYMENT.md`, `docs/README.md` — top-level deployment notes and the
  documentation index.

## Consequences

### Positive

1. **No orphaned documentation**: every remaining file describes something
   that currently exists in this repository.
2. **Simpler mental model**: one tree, organized by topic, rather than two
   tiers whose distinction no longer maps to a real audience split.
3. **Discoverability preserved**: `docs/README.md` remains the single index
   linking all guides and references.

### Negative

1. **Lost onboarding narrative**: a genuinely new contributor loses the
   guided "first CI pass" walkthrough the template tier provided; mitigated
   by `README.md`'s own Development section and `CONTRIBUTING.md`.

### Neutral

1. The topic subdirectories (`docs/workflows/`, `docs/security/`,
   `docs/distribution/`, `docs/testing/`, `docs/ux/`) are preserved unchanged
   from the upstream template's structure.
2. `docs/adr/` remains at its existing location.

## Decision Outcome

The decision achieves its primary objective — a documentation tree with no
orphaned content — measured by: every file under `docs/` describes a workflow,
process, or component that exists in this repository as of this ADR's date.

## Related Decisions

- [ADR-0001: Use Architectural Decision Records](0001-use-architectural-decision-records.md) — establishes the ADR practice this document follows.

## Links

- [attested-delivery/rust-template](https://github.com/attested-delivery/rust-template) — the upstream attested-delivery template `mif-rs` was forked from, whose two-tier `docs/template/` + `docs/runbooks/` split this ADR moves away from.

## More Information

- **Date**: 2026-07-02
- **Source**: workspace bootstrap (this repository's initial setup)

## Audit

### 2026-07-02

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| docs/template/ and docs/observability/ removed, structure documented as current state | docs/ | - | accepted |

**Summary:** Documentation structure decision recorded to match the actual
post-bootstrap `docs/` tree, superseding the upstream template's two-tier
design without a separate superseding ADR since the original decision was
never accepted in this repository's own history — it is being corrected at
first documentation of this repo's actual state.

**Action Required:** None — this ADR documents current, already-adopted structure.
