---
title: "Use Architectural Decision Records"
description: "Adopt Structured MADR ADRs, stored in docs/adr/ and reviewed via pull request, to capture mif-rs's architecturally significant decisions instead of leaving them undocumented in commit messages or Slack threads."
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
---

# ADR-0001: Use Architectural Decision Records

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs` is a 5-crate Cargo workspace (`mif-core`, `mif-schema`, `mif-ontology`,
`mif-cli`, `mif-mcp`) with a strict dependency chain, a signed/attested release
pipeline forked from `attested-delivery/rust-template`, and org-specific
security-gate wiring. As the workspace grows, decisions with real, hard-to-reverse
consequences — schema vendoring vs. runtime fetch, hand-written vs. generated
types, which crates get workspace-level lint tables — need a durable record, or
the reasoning behind them is lost the moment the PR that made them is no longer
top-of-mind.

> **Editorial note (2026-07-03):** the "5-crate" count above reflects the
> workspace's state when this ADR was written (2026-07-02). The workspace has
> since grown to 9 members (`mif-core`, `mif-problem`, `mif-schema`,
> `mif-frontmatter`, `mif-ontology`, `mif-embed`, `mif-store`, `mif-cli`,
> `mif-mcp`) — see the root `Cargo.toml`. This note corrects the stale crate
> count for readers; it does not amend the decision or its rationale, which
> stand as originally recorded above.

### Current Limitations

1. **No durable decision record**: rationale currently lives only in commit
   messages and PR descriptions, which are hard to search and easy to lose track
   of once a PR is merged and closed.
2. **Repeated re-litigation risk**: without a citable record, a future
   contributor (human or agent) can re-open a settled question because they have
   no way to discover it was already decided and why.
3. **No machine-readable decision trail**: an agent working on this repo has no
   structured way to check "is this decision still valid" or "what superseded
   it" short of reading full git history.

## Decision Drivers

### Primary Decision Drivers

1. **Low overhead**: the format shall not require tooling beyond what this
   repository already has (git, PR review, markdown) to author or review.
2. **Durable and versioned**: decisions shall live alongside the code they
   govern, reviewed the same way code changes are reviewed.
3. **Machine-readable trail**: since `mif-rs` implements the MIF specification
   itself, its own ADRs should be genuine MIF documents — dogfooding the format
   rather than treating documentation as a separate, unstructured concern.

### Secondary Decision Drivers

1. **Low learning curve**: contributors should be able to read and write an ADR
   without external tooling beyond a markdown editor.

## Considered Options

### Option 1: No formal decision record (status quo)

**Description**: Rely on commit messages, PR descriptions, and code comments to
carry decision rationale, with no dedicated document type.

**Advantages**:

- Zero process overhead; nothing new to learn or maintain.

**Disadvantages**:

- Rationale is scattered and hard to search; PR descriptions are not indexed as
  decisions.
- No way to mark a decision superseded or track its current validity.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: None.
- **Ecosystem Risk**: High. Knowledge loss compounds as the workspace grows.

### Option 2: A wiki or external docs site per decision

**Description**: Record decisions in a GitHub wiki page or an external docs
site, separate from the code repository.

**Advantages**:

- Free-form; no structural constraints.

**Disadvantages**:

- Not versioned alongside the code; drifts out of sync with what actually
  shipped. Not reviewed through the same PR process as code.

**Disqualifying Factor**: a decision record that isn't reviewed and versioned
with the code it governs is exactly the failure mode this decision is meant to
prevent.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Medium. External docs rot independently of the codebase.

### Option 3: Architectural Decision Records (Structured MADR / MIF)

**Description**: Store one markdown file per decision under `docs/adr/`,
numbered sequentially, reviewed via pull request like any code change, using the
Structured MADR format (which projects losslessly to MIF JSON-LD).

**Advantages**:

- Versioned and reviewed alongside code, with zero new tooling required beyond
  markdown and PR review.
- Machine-readable frontmatter (status, relationships, provenance) makes a
  decision's current validity and supersession chain queryable, not just
  readable.
- Dogfoods the MIF format this repository itself implements.

**Disadvantages**:

- Requires discipline to write an ADR when a decision is made, not after the
  fact.

**Risk Assessment**:

- **Technical Risk**: Low. Plain markdown, no new infrastructure.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

## Decision

We adopt **Architectural Decision Records**, in the Structured MADR / MIF
format, to document significant architectural decisions in `mif-rs`.

ADRs will:

- Be stored in the `docs/adr/` directory.
- Be numbered sequentially (0001, 0002, …).
- Include the full Structured MADR section set: Status, Context, Decision
  Drivers, Considered Options (each with a Risk Assessment), Decision,
  Consequences, Decision Outcome, and Audit.
- Be reviewed through pull requests like any other code change.
- Carry MIF frontmatter (`conceptType`, `status`, `created`/`updated`,
  `author`, `project`) so the decision record is genuinely machine-readable,
  not just human-readable prose.

## Consequences

### Positive

1. **Transparency**: architectural decisions are documented and discoverable,
   not buried in closed PRs.
2. **Context preservation**: future contributors (human or agent) understand
   why a decision was made, not just what it was.
3. **Machine-readable trail**: an agent can check a decision's `status` and
   `relationships` instead of re-deriving it from git history.

### Negative

1. **Overhead**: requires discipline to write an ADR when a decision is made,
   not retroactively; mitigated by keeping the format lightweight (markdown,
   no external tooling).
2. **Learning curve**: contributors need to learn the Structured MADR section
   structure; mitigated by `templates/good.md` in the `mif-docs:adr` skill and
   this ADR itself serving as a worked example.

### Neutral

1. Not every decision needs an ADR — only architecturally significant ones
   with real alternatives that were weighed.
2. An ADR's `status` changes over time (`proposed` → `accepted` →
   `deprecated`/`superseded`); an accepted ADR's outcome is not edited in
   place — a decision that changes gets a new, superseding ADR.

## Decision Outcome

The decision achieves its primary objective — a durable, reviewable,
machine-readable decision trail — measured by: every architecturally
significant decision in `mif-rs` from this point forward has a corresponding
ADR under `docs/adr/`, reviewed via PR.

## Related Decisions

None — this is the first ADR in this repository; it establishes the practice
every subsequent ADR follows.

## Links

- [Structured MADR specification](https://github.com/modeled-information-format/structured-madr)
- [MIF (Modeled Information Format) specification](https://mif-spec.dev)

## More Information

- **Date**: 2026-07-02
- **Source**: workspace bootstrap (this repository's initial setup)

## Audit

### 2026-07-02

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| ADR format adopted at workspace bootstrap | docs/adr/ | - | accepted |

**Summary:** Decision adopted at initial workspace setup; no prior alternative
was in active use to migrate away from.

**Action Required:** None — this ADR documents current, already-adopted practice.
