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
- [ADR-0003](0003-virtual-cargo-workspace.md) — Virtual Cargo Workspace, Not a Root Package
- [ADR-0004](0004-libraries-never-depend-on-binaries.md) — Library Crates Never Depend on the Binary Crates
- [ADR-0005](0005-per-crate-thiserror-error-enums.md) — Per-Crate thiserror Error Enums, No Shared Top-Level Error Type
- [ADR-0006](0006-vendor-json-schema-at-compile-time.md) — Vendor the Canonical JSON Schema at Compile Time, Not Fetch at Validate Time
- [ADR-0007](0007-generic-frontmatter-passthrough.md) — Generic Frontmatter Pass-Through, Not a Curated Field List
- [ADR-0008](0008-hand-written-core-types-not-codegen.md) — Hand-Written Core Types, Not Schema-to-Rust Codegen
- [ADR-0009](0009-pedantic-clippy-lint-groups.md) — Pedantic, Nursery, and Cargo Clippy Lint Groups, Workspace-Wide
- [ADR-0010](0010-release-profile-panic-abort.md) — Release Profile: `panic = "abort"`, strip, and Thin LTO
- [ADR-0011](0011-ban-openssl-and-atty.md) — Ban openssl and atty; Use rustls and std::io::IsTerminal
- [ADR-0012](0012-cargo-chef-docker-layer-caching.md) — cargo-chef Multi-Stage Docker Build for Dependency-Layer Caching
- [ADR-0013](0013-chainguard-glibc-dynamic-container-base.md) — Chainguard glibc-dynamic as the Container Runtime Base, Superseding distroless/cc-debian12
- [ADR-0014](0014-ghcr-package-visibility-manual-process.md) — GHCR Package Visibility: Manual, Not Automated
- [ADR-0015](0015-local-embeddings-sqlite-vector-store.md) — Local Embeddings and a SQLite Brute-Force Vector Store
- [ADR-0016](0016-lefthook-ci-parity-git-hooks.md) — Lefthook Git Hooks: Fast Pre-Commit, Full CI-Parity Pre-Push
- [ADR-0017](0017-rfc9457-error-uri-hosted-on-pages.md) — RFC 9457 Error-Type URIs Hosted on This Repository's Own GitHub Pages Site
- [ADR-0018](0018-rustdoc-and-starlight-unified-pages-deployment.md) — Publish rustdoc Alongside the Starlight Site in One Pages Deployment
