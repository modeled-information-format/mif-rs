---
title: "Local Embeddings and a SQLite Brute-Force Vector Store"
description: "Give mif-cli and mif-mcp semantic search over ingested MIF documents via local, CPU-only embedding inference (mif-embed) and a SQLite-backed brute-force cosine-similarity vector store (mif-store), instead of an external embedding API or a dedicated vector database."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - embeddings
  - vector-store
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
  - 0004-libraries-never-depend-on-binaries.md
---

# ADR-0015: Local Embeddings and a SQLite Brute-Force Vector Store

## Status

Accepted

## Context

### Background and Problem Statement

`mif-embed` and `mif-store` were added on 2026-07-03 (commit `5c0a416`, PR #6,
"feat(ingest): add MIF document ingestion, embedding, and semantic search") to
give `mif-cli` and `mif-mcp` semantic search over ingested MIF documents.
`mif-embed` loads `sentence-transformers/all-MiniLM-L6-v2` via `candle` for
local, CPU-only inference, fetching the model from the Hugging Face Hub once
and caching it under the platform cache directory so subsequent runs are
fully offline. `mif-store` is a `SQLite`-backed (`rusqlite`, bundled) vector
store doing brute-force cosine-similarity ranking over stored embeddings,
rather than an approximate-nearest-neighbor (ANN) index.

**No comparison against alternatives exists anywhere for this decision.** The
commit history, the PR description, and the module documentation for both
crates describe *what* was built in detail — the model, the caching path,
the on-disk vector blob format, the target corpus scale — but none of them
records a considered-alternatives comparison against an external embedding
API, a dedicated vector database, or an ANN index. This ADR is being written
after the fact, and the rationale below is **reconstructed** from this
workspace's own stated, repeated design values found elsewhere in this
codebase — chiefly `mif-schema`'s explicit offline-only validation design
(see [ADR-0006: Vendor the Canonical JSON Schema at Compile Time, Not Fetch
at Validate Time](0006-vendor-json-schema-at-compile-time.md)) and
`mif-embed`'s own module doc comment, which describes the crate as loading
its model "on first use, caching the model files under the platform cache
directory so later runs are offline." It is not a transcription of an
original decision record, because none exists.

### Current Limitations

1. **No original alternatives comparison to cite**: unlike ADR-0006, which
   documents an already-considered rejection of network-fetched schema
   validation, no equivalent record exists for the embedding/vector-store
   choice. This ADR fills that gap retroactively rather than quoting a prior
   decision.
2. **Two architecturally significant choices bundled into one PR**: PR #6
   made both the local-inference choice (`mif-embed`) and the brute-force
   SQLite choice (`mif-store`) without a documented rationale for either,
   leaving both open to reasonable-sounding but uninformed re-litigation.

## Decision Drivers

### Primary Decision Drivers

1. **Offline-first precedent**: `mif-cli` and `mif-mcp` are meant to work
   offline after first use, consistent with the offline-first precedent this
   workspace has already established for schema validation (ADR-0006), which
   has zero network dependency at validate time.
2. **No multi-tenant or high-QPS requirement**: a CLI/MCP tool operating over
   one user's local document corpus has no natural multi-tenant or
   high-queries-per-second requirement that would justify operating a
   separate database service.
3. **Single, self-contained binaries**: both `mif-cli` and `mif-mcp` are
   meant to remain single, self-contained executables with no separate
   service to deploy or operate.

### Secondary Decision Drivers

1. **Corpus scale is small and bounded**: `mif-store`'s own module
   documentation targets "a few thousand rows" as its expected corpus scale,
   not a scale that requires specialized indexing infrastructure.

## Considered Options

> **Reconstructed, not historically documented.** The options and risk
> assessments below were not weighed in the original PR; they are
> constructed here, after the fact, from this workspace's own stated design
> values (see Context) to give this decision a citable rationale going
> forward. Treat this section as this ADR's own reasoning, not as a record
> of what was actually debated in PR #6.

### Option 1: Call an external embedding API at ingest/search time

**Description**: Call a hosted embeddings endpoint (e.g. an external
provider's embeddings API) over the network at ingest and search time,
instead of running inference locally.

**Advantages**:

- No local model to fetch, cache, or run — `mif-cli`/`mif-mcp` would carry no
  `candle`/`tokenizers`/`hf-hub` dependency footprint at all.
- Access to larger, more capable embedding models than what runs efficiently
  on CPU-only local inference.
- The provider owns model upgrades; this workspace would never need to
  re-vendor or re-cache a new model version itself.

**Disadvantages**:

- Requires network access and an API credential for every single use,
  breaking the offline-after-first-fetch precedent this workspace has
  already established for schema validation (ADR-0006).
- Adds per-call latency and cost that a local-first CLI tool does not need.

**Risk Assessment**:

- **Technical Risk**: Medium. Correctness and availability become dependent
  on network state and a third-party service's uptime.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: High. Breaks entirely offline or in airgapped
  environments, the same failure mode ADR-0006 already rejected for schema
  validation.

### Option 2: Local, CPU-only inference via candle plus a SQLite brute-force vector store (chosen)

**Description**: Run local, CPU-only inference via `candle` with
`sentence-transformers/all-MiniLM-L6-v2`, fetched once from the Hugging Face
Hub and cached under the platform cache directory so all later runs are
fully offline. Pair it with a `SQLite`-backed vector store (`mif-store`)
doing brute-force cosine-similarity ranking, rather than an
approximate-nearest-neighbor index.

**Advantages**:

- Fully offline after the one-time model fetch, consistent with this
  workspace's established offline-first precedent.
- No separate service to deploy or operate; both `mif-cli` and `mif-mcp`
  remain single, self-contained executables.
- `mif-store`'s own module documentation states its target scale is "a few
  thousand rows," where brute force is both simpler to implement and
  maintain and fast enough — no ANN index is warranted at that scale.

**Disadvantages**:

- CPU-only local inference is slower per call than a hosted API or a
  GPU-backed service.
- Brute-force ranking is O(n) per query, which does not scale past the
  corpus sizes this crate currently targets.

**Risk Assessment**:

- **Technical Risk**: Low at current target scale ("a few thousand rows,"
  per `mif-store`'s own module documentation); rises if corpus sizes grow
  well beyond that.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low. No external service dependency, no credential
  management, no network dependency after first fetch.

### Option 3: A dedicated, separately-operated vector database

**Description**: Use a dedicated, separately-operated vector database — for
example a service like Qdrant, or a Postgres extension like `pgvector` —
instead of a brute-force store embedded in the binary.

**Advantages**:

- Purpose-built approximate-nearest-neighbor (ANN) indexing (e.g. HNSW) scales
  past the O(n)-per-query limit of brute-force cosine-similarity ranking.
- Mature, well-understood technology with established operational tooling.
- Would readily support a multi-tenant or high-QPS workload, if either
  requirement ever emerged for `mif-cli`/`mif-mcp`.

**Disadvantages**:

- Requires running and operating a separate service, directly at odds with
  the single-self-contained-binary goal both `mif-cli` and `mif-mcp` are
  meant to satisfy.
- Disproportionate at the corpus scale `mif-store`'s own documentation
  targets ("a few thousand rows") — the operational overhead of a separate
  database service is not justified by that scale.

**Risk Assessment**:

- **Technical Risk**: Low in isolation (mature, well-understood technology),
  but High for this workspace's goals specifically, since it reintroduces a
  service dependency this workspace has otherwise avoided.
- **Schedule Risk**: Medium. Standing up and maintaining a separate service
  is nontrivial additional work for no benefit at current scale.
- **Ecosystem Risk**: High. Breaks the single-self-contained-binary
  distribution model both `mif-cli` and `mif-mcp` rely on.

## Decision

We use **local, CPU-only embedding inference** (`mif-embed`, via `candle`
and `sentence-transformers/all-MiniLM-L6-v2`) paired with a **SQLite-backed
brute-force cosine-similarity vector store** (`mif-store`), rather than an
external embedding API or a dedicated vector database.

## Consequences

### Positive

1. **Fully offline after one-time setup**: no per-query API cost or
   credential requirement, and no separate service to operate.
2. **Consistent with this workspace's broader offline-first design theme**,
   already established for schema validation (ADR-0006).

### Negative

1. **Slower per-call inference**: CPU-only local inference is slower than a
   hosted API call or a GPU-backed service for very large corpora.
2. **O(n) query scaling**: brute-force ranking is O(n) per query — this
   design will need real revisiting if corpus sizes grow well beyond the "a
   few thousand rows" scale `mif-store`'s own module documentation describes
   as its target.

### Neutral

1. **Private on-disk vector format**: the vector blob's on-disk storage
   format (raw little-endian `f32` components, documented in `mif-store`'s
   own module doc comment) is a private, internal format this crate alone
   reads and writes — not a public interchange format — so it can change in
   a future version without an external compatibility concern.

## Decision Outcome

The decision achieves its primary objective — offline operation after a
one-time model fetch — measured by: `mif-cli` and `mif-mcp`'s ingest/search
operations succeed with zero network access after the embedding model has
been fetched once, consistent with `mif-embed`'s own module doc comment,
which states the model is loaded "from the Hugging Face Hub on first use,
caching the model files under the platform cache directory so later runs are
offline. Inference runs on CPU only."

The O(n) brute-force scaling limit is explicitly acknowledged here as a
known, accepted limitation at current target scale, not a solved problem —
`mif-store`'s own module documentation states brute force is adequate at "a
few thousand rows," and this ADR does not claim it scales beyond that.

## Related Decisions

- [ADR-0004: Library Crates Never Depend on the Binary Crates](0004-libraries-never-depend-on-binaries.md)
- [ADR-0006: Vendor the Canonical JSON Schema at Compile Time, Not Fetch at Validate Time](0006-vendor-json-schema-at-compile-time.md) — the offline-first precedent this decision's reconstructed rationale draws on.

## Links

- [sentence-transformers/all-MiniLM-L6-v2](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2) — the embedding model `mif-embed` loads via `candle`, fetched once from the Hugging Face Hub and cached under the platform cache directory.
- [candle](https://github.com/huggingface/candle) — the Rust ML framework `mif-embed` uses for local, CPU-only inference (`candle-core`, `candle-nn`, `candle-transformers`).
- [SQLite documentation](https://www.sqlite.org/docs.html) — the embedded database `mif-store` uses (via `rusqlite`, bundled) for on-disk vector storage.
- [Qdrant](https://qdrant.tech/documentation/) — representative of the dedicated, separately-operated vector database class of alternative considered and rejected in Option 3.

## More Information

- **Date**: 2026-07-03
- **Source**: commit `5c0a416` / PR #6 ("feat(ingest): add MIF document
  ingestion, embedding, and semantic search"), and the module doc comments in
  `crates/mif-embed/src/lib.rs` and `crates/mif-store/src/lib.rs`.

## Audit

### 2026-07-03

**Status:** Partial

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Local CPU-only inference via candle, model cached under platform cache dir | crates/mif-embed/src/lib.rs | 1-7 | accepted |
| SQLite-backed brute-force cosine-similarity ranking, no ANN index | crates/mif-store/src/lib.rs | 1-16, 392-403 | accepted |
| Considered Options section is reconstructed rationale, not a historically documented alternatives comparison | docs/adr/0015-local-embeddings-sqlite-vector-store.md | Considered Options | reconstructed, not original |

**Summary:** The implementation described (local candle inference,
SQLite brute-force vector store) is accurately documented and verified
against the actual module doc comments. The status is **Partial**, not
Compliant, specifically because the Considered Options section is
reconstructed rationale built from this workspace's own stated design
values elsewhere, not a transcription of an original, historically
documented alternatives comparison — none exists in the commit, the PR
description, or the module documentation for this feature. Per this
suite's own honesty requirements for ADRs, a reconstructed position that is
labeled as such is a real, valid position for this section to hold;
presenting it silently as original historical fact would not be.

**Action Required:** None — this ADR documents current, already-adopted
practice, with its reconstructed-rationale status disclosed above.
