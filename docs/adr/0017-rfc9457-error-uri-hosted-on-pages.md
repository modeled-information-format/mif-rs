---
title: "RFC 9457 Error-Type URIs Hosted on This Repository's Own GitHub Pages Site"
description: "Moves mif_problem's RFC 9457 error-type base URI from the unpublished mif-spec.dev/errors namespace to this repository's own GitHub Pages site, making every emitted problem-type URI genuinely dereferenceable."
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
  - documentation
  - github-pages
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0005-per-crate-thiserror-error-enums.md
  - 0018-rustdoc-and-starlight-unified-pages-deployment.md
---

# ADR-0017: RFC 9457 Error-Type URIs Hosted on This Repository's Own GitHub Pages Site

## Status

Accepted

## Context

### Background and Problem Statement

`mif_problem::ERROR_TYPE_BASE_URI` originally pointed at
`https://mif-spec.dev/errors`. Its own doc comment stated explicitly that this
was "an identifier namespace, not a claim that a reference page is published
at that path today." No reference page was ever actually published at that
location — the URI existed purely as a stable identifier string embedded in
every emitted RFC 9457 problem-type URI (e.g.
`https://mif-spec.dev/errors/invalid-document/v1`), across all ~35 unique
problem types this workspace's 7 crates (`mif-schema`, `mif-ontology`,
`mif-frontmatter`, `mif-embed`, `mif-store`, `mif-cli`, `mif-mcp`) emit.

As `mif-rs` grew a Pages-deployed documentation site (this same session,
merged as pull request #11), the question became concrete: should these 35
problem types get real, dereferenceable reference pages, and if so, where
should the identifier namespace actually point?

### Current Limitations

1. **Dead links by design**: every one of the 35 emitted `type` URIs was a
   permanent dead link — RFC 9457 recommends, though does not strictly
   require, that a problem type's URI be dereferenceable to a real
   description of that type.
2. **No home for implementation-level error docs**: `mif-spec.dev` serves the
   normative MIF specification; it has no natural place for one
   implementation's own tooling/error catalog.
3. **Any future fix must not require a second migration**: whatever URI is
   chosen has to remain stable and versioned, since re-pointing it again
   later is itself a breaking change to every consumer parsing these URIs as
   identifiers.

## Decision Drivers

### Primary Decision Drivers

1. **Dereferenceability**: an RFC 9457 type URI SHOULD resolve to a real,
   human-and-machine-readable page describing that problem type.
2. **Correct domain ownership**: `mif-spec.dev` is reserved for the normative
   MIF specification itself, not one implementation's own error catalog —
   conflating the two would blur what `mif-spec.dev` actually represents.

### Secondary Decision Drivers

1. **Long-term stability**: the chosen URI must remain stable and versioned
   over the long term; changing it again later is itself a breaking change to
   every consumer parsing these URIs as identifiers.

## Considered Options

### Option 1: Keep `https://mif-spec.dev/errors/...` as a pure, never-dereferenced identifier namespace (status quo)

**Description**: Leave `ERROR_TYPE_BASE_URI` unchanged; continue treating it
as an opaque identifier string with no real page behind it.

**Advantages**:

- Zero implementation effort.

**Disadvantages**:

- Every one of the 35 emitted URIs is a permanent dead link for any human or
  agent that tries to actually visit it — a poor experience that directly
  contradicts RFC 9457's own recommendation that type URIs be dereferenceable
  where practical.

**Risk Assessment**:

- **Technical Risk**: Low. No change means no chance of a new defect.
- **Schedule Risk**: None.
- **Ecosystem Risk**: High. Every consumer that follows a `type` URI hits a
  dead link, permanently.

### Option 2: Host an `/errors/` section on the separate MIF specification repository

**Description**: Request that the separate MIF specification repository add
a `/errors/` section specifically for `mif-rs`'s own implementation-level
error catalog.

**Advantages**:

- Places every MIF-related dereferenceable URI under a single,
  already-established domain that consumers of the broader MIF ecosystem
  already recognize.
- No new Pages site or deployment pipeline to stand up in `mif-rs` itself —
  reuses infrastructure that already exists.

**Disadvantages**:

- Couples `mif-rs`'s own release cadence and content to a separate
  repository's deploy pipeline and review process.
- Conflates the normative MIF specification (what `mif-spec.dev` exists to
  serve) with one specific implementation's own tooling error catalog, which
  is not normative spec content at all.

**Disqualifying Factor**: hosting implementation-level error documentation on
the normative spec's own domain blurs what that domain represents, for every
future reader of `mif-spec.dev`.

**Risk Assessment**:

- **Technical Risk**: Medium. Cross-repository coordination for every future
  problem type added.
- **Schedule Risk**: Medium. Every change requires review in a repository
  `mif-rs` does not control.
- **Ecosystem Risk**: Medium. Conflates normative spec content with one
  implementation's error catalog.

### Option 3: Self-host at this repository's own GitHub Pages site (chosen)

**Description**: Change `ERROR_TYPE_BASE_URI` to
`https://modeled-information-format.github.io/mif-rs/references/errors`, and
publish a real reference page at `docs/references/errors/{slug}/{version}.md`
for each problem type this workspace emits.

**Advantages**:

- The identifier namespace and its actual dereferenced content now live in
  the same repository, released and versioned together.
- No coupling to a separate repository's release cadence or review process.
- Every emitted `type` URI is genuinely dereferenceable.

**Disadvantages**:

- Breaking change to the literal string value of every one of the ~35
  emitted problem-type URIs; any consumer that treated the old
  `mif-spec.dev`-shaped string as a fixed literal rather than an opaque
  identifier breaks.
- Ties the error reference pages' availability to `mif-rs`'s own GitHub
  Pages deployment rather than the more established `mif-spec.dev` domain.

**Risk Assessment**:

- **Technical Risk**: Low. The Pages site already exists (PR #11); adding
  reference pages is additive.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low. Breaking change to the URI's literal string value,
  but at v0.1.0 with no established external consumers (see Consequences).

## Decision

We change `mif_problem::ERROR_TYPE_BASE_URI` to
`https://modeled-information-format.github.io/mif-rs/references/errors`,
self-hosting the RFC 9457 problem-type reference pages on this repository's
own GitHub Pages site.

Implemented as:

- `mif_problem::ERROR_TYPE_BASE_URI` now derives every type URI as
  `{ERROR_TYPE_BASE_URI}/{slug}/{version}`.
- `docs/references/errors/{slug}/{version}.md` publishes a real,
  dereferenceable reference page for each of the 35 unique problem types this
  workspace emits.
- Some slugs — `io`, `invalid-json`, `document-not-found` — are intentionally
  shared across multiple crates that emit the identical RFC 9457 problem
  shape; the `instance` field, not the `type` URI, is what disambiguates
  which crate actually produced a given occurrence, via its
  `urn:{crate_name}:{slug}` format.

## Consequences

### Positive

1. **Genuine dereferenceability**: every emitted type URI now resolves to a
   real page describing that exact problem: its HTTP status, exit code,
   message template, cause, and suggested fix — for a human reading a raw
   JSON error envelope, or an agent programmatically following the `type`
   field.
2. **No cross-repository coupling**: reference-page publication ships on
   `mif-rs`'s own release cadence, with no dependency on a separate
   repository's deploy pipeline or review process.

### Negative

1. **Breaking change to the literal URI string**: this changes the literal
   string value of every one of the ~35 emitted problem-type URIs. Any
   consumer that had begun depending on the old `mif-spec.dev`-shaped string
   as a literal value — rather than treating it as an opaque,
   potentially-changing identifier, which RFC 9457 itself recommends — would
   break. This is judged low-impact in practice: the URI was explicitly documented
   as "not a claim a reference page is published" before this change, and
   `mif-rs` is at v0.1.0 with no established external consumers depending on
   the old literal string.

### Neutral

1. The per-type `version` segment (currently uniformly `v1` across all 35
   types) is the actual stability commitment, independent of both the
   crate's own semver and of this hosting-location decision — a future
   breaking change to one specific problem type's meaning would ship as a
   new version segment (e.g. `/v2`) for that one type, not require touching
   `ERROR_TYPE_BASE_URI` again.

## Decision Outcome

The decision achieves its primary objective — every emitted problem-type URI
is genuinely dereferenceable — measured by: every one of the 35 problem
types catalogued in `docs/references/errors/index.md` has a `type` URI that,
when visited at
`https://modeled-information-format.github.io/mif-rs/references/errors/{slug}/{version}/`,
resolves to that exact page. `docs/references/errors/index.md`'s own catalog
listing lists all 35 (2 schema-validation + 6 ontology-resolution + 10
frontmatter-projection + 9 embedding + 6 vector-store + 2 CLI/MCP-specific =
35, with `io` shared between `mif-ontology` and `mif-cli`/`mif-mcp` counted
once).

## Related Decisions

- [ADR-0005: Per-Crate `thiserror` Error Enums](0005-per-crate-thiserror-error-enums.md) — establishes the per-crate error-enum pattern that each implements `ToProblem` against, producing the type URIs this decision hosts.
- [ADR-0018: Rustdoc and Starlight Unified Pages Deployment](0018-rustdoc-and-starlight-unified-pages-deployment.md) — establishes the GitHub Pages site this decision hosts the error reference pages on.

## Links

- [RFC 9457: Problem Details for HTTP APIs](https://www.rfc-editor.org/rfc/rfc9457)

## More Information

- **Date**: 2026-07-03
- **Source**: this session's work, merged as pull request #11 to this
  repository; see `crates/mif-problem/src/lib.rs`'s `ERROR_TYPE_BASE_URI`
  doc comment for the current, in-code statement of this decision.

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| `ERROR_TYPE_BASE_URI` doc comment states: "Every implementer's `type` URI is derived as `{ERROR_TYPE_BASE_URI}/{slug}/{version}` ... and is dereferenceable: `docs/references/errors/{slug}/{version}.md` publishes a real reference page at that path via this repo's GitHub Pages site. `mif-spec.dev` is reserved for the normative MIF specification itself, not this implementation's own tooling/error reference — hence the repo-scoped Pages URL rather than the spec domain." | `crates/mif-problem/src/lib.rs` | 28-39 | accepted |
| `docs/references/errors/index.md` catalogs all 35 published reference pages across the 7 crates that emit RFC 9457 problem types | `docs/references/errors/index.md` | - | accepted |

**Summary:** Verified against the current repository state: `ERROR_TYPE_BASE_URI` is set to `https://modeled-information-format.github.io/mif-rs/references/errors` in code, and `docs/references/errors/index.md` lists 35 `{slug}/{version}` reference pages, matching the count this decision commits to.

**Action Required:** None — this ADR documents current, already-implemented practice.
