---
title: "Passage-Level Embedding and Best-Passage Search"
description: "Embed each MIF document as a summary passage plus heading-scoped body passages, store them beside the document vector, and rank search results by each document's best-matching passage, amending ADR-0015's one-vector-per-document design."
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
  - search
  - architecture
status: accepted
created: 2026-11-24
updated: 2026-11-24
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0015-local-embeddings-sqlite-vector-store.md
  - 0004-libraries-never-depend-on-binaries.md
---

# ADR-0021: Passage-Level Embedding and Best-Passage Search

## Status

Accepted. Amends [ADR-0015: Local Embeddings and a SQLite Brute-Force Vector
Store](https://modeled-information-format.github.io/mif-rs/adr/0015-local-embeddings-sqlite-vector-store/):
the model, local inference, the SQLite store, and brute-force ranking all
stand. What changes is what is embedded for a document and what `search`
ranks.

## Context

### Background and Problem Statement

Through release 0.11.0, `ingest` embedded one vector per document from the
document's body, and the tokenizer truncated that body to the model's
512-token input window. `search` ranked those vectors by cosine similarity.

A corpus of templated records exposed the limit of that design. In an
enterprise-architecture vault of about 1,300 documents, every capability and
constraint record opens with the same kind of text: a purpose line, an owner
line, usage notes, and a list of related links. That text fills the window.
The part of a record that sets it apart, its definition, sits past the
window and was never embedded.

Measured on 2026-11-24 against that vault with `mif-cli` 0.11.0: searching
with a record's own definition returned that record first for 20% of 115
capability records and 19% of 129 constraint records, and in the top ten for
54% and 57%. Searching with a record's name returned it in the top three
for 52% and 50%.

### Current Limitations

1. **The embedded text is whatever comes first.** Nothing selects the text
   that identifies a document; truncation selects the opening of the body.
2. **The title, description, and entity fields are not embedded at all.**
   Only `content` reaches the model.
3. **The model is used past its training length.** `all-MiniLM-L6-v2` was
   trained on inputs of up to 256 tokens; quality drops between there and
   the 512-token window.
4. **A hit cannot say where it matched.** One vector per document carries
   no location.

## Decision Drivers

### Primary Decision Drivers

1. **A document must be findable by the text that identifies it**, wherever
   that text sits in the document.
2. **No change to the MIF document format.** Existing documents stay valid,
   and their `EmbeddingReference` stays meaningful.
3. **`mif-rh` must not change.** Its findings index and stored
   `stamped-quantile-v1` calibrations depend on `Embedder::embed` producing
   the vectors it produces today.
4. **Existing vector stores and existing callers keep working.** The
   Obsidian plugin and scripts parse `mif-cli`'s output and share its
   database files, and may run an older binary beside a newer one.

### Secondary Decision Drivers

1. **The offline, single-binary, brute-force design of ADR-0015 stays.**
2. **A hit should name the section that matched**, so a caller can cite it.

## Considered Options

### Option 1: Keep one vector per document, and embed a better text

**Description**: Build one text per document from the title, description,
and entity definition, followed by as much body as fits, and embed that.

**Advantages**:

- No schema change; one row per document as before.
- Fixes the worst case: the definition is embedded.

**Disadvantages**:

- Everything outside the chosen text is still unsearchable. A query that
  matches a later section finds nothing.
- One vector must average the summary and whatever body fits, which blurs
  both.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low. But it solves name and definition queries only.

### Option 2: A summary passage plus heading-scoped body passages, ranked by best passage (chosen)

**Description**: Split each document into passages of at most 256 tokens: a
summary passage (title, entity name, description, and the entity definition
or first statement), then the body split at headings, each passage prefixed
with the title and heading. Store every passage vector, and rank each
document by its best-matching passage.

**Advantages**:

- Every part of a document is searchable, and the summary passage answers
  name and definition queries directly.
- Each passage is within the model's training length.
- A hit reports the heading of the passage that matched.

**Disadvantages**:

- About twenty times as many vectors to store and to score per query.
- Ingest embeds every passage, so a full ingest takes minutes rather than
  seconds.

**Risk Assessment**:

- **Technical Risk**: Low. The store stays brute-force; the passage builder
  is plain text handling with no new dependency.
- **Schedule Risk**: Medium. It touches the embedder, the store schema, and
  both binaries.
- **Ecosystem Risk**: Low, given the compatibility measures in the Decision.

### Option 3: Change the embedding model

**Description**: Replace `all-MiniLM-L6-v2` with a model that has a longer
input window or better retrieval quality, keeping one vector per document.

**Advantages**:

- A longer window reaches more of each document.
- No schema change.

**Disadvantages**:

- A long document averaged into one vector still loses its specifics; a
  longer window moves the limit without removing it.
- Every stored vector, every document's `EmbeddingReference`, and every
  `mif-rh` calibration would have to be regenerated.

**Risk Assessment**:

- **Technical Risk**: Medium. Retrieval quality on this kind of corpus is
  unknown until measured.
- **Schedule Risk**: Medium.
- **Ecosystem Risk**: High. It changes vectors for every consumer at once,
  including `mif-rh`.

## Decision

We embed each document as **passages** and rank `search` results by each
document's **best-matching passage**.

- `mif_embed::passages` builds the passages: the summary first, then the
  body split at headings of level 1 to 3. Link targets, bare URLs,
  `**Related:**` lines, table rule rows, and the `Relationships` and
  `Navigation` sections are left out. A long section is split at paragraph
  or table-row boundaries with a 32-token overlap.
- The 256-token limit is enforced by the passage builder. The tokenizer's
  truncation and `Embedder::embed` are unchanged, so `mif-rh` is unaffected.
- `mif-store` gains a `passages` table and nullable `source` and `namespace`
  columns, recorded as schema version 1. The `embeddings` table keeps one
  row per document; that row's vector is the summary passage.
- `VectorStore::search` ranks by best passage and reports its heading.
  `top_k_similar` keeps ranking document vectors, so `find-similar` stays
  document to document.
- Passage vectors are internal to the store. They are not part of the MIF
  specification, and a document's `EmbeddingReference` keeps pointing at
  its one document vector.

Compatibility is part of the decision:

- A 0.11.0 database is upgraded in place on open with no row rewritten, and
  stays usable by 0.11.0. A read-only database is opened as it is.
- A document with no passages is ranked by its document vector, so a store
  that is only partly re-ingested stays searchable.
- Each passage carries its document's content hash. Passages left behind
  when an older binary stores a changed document no longer match and are
  ignored.
- `search` prints `score  id` as before; the heading is opt-in
  (`--show-passage`). The `ingest` and `corpus-stats` lines only gain
  fields.

## Consequences

### Positive

1. **Records are found by their own text.** See Decision Outcome.
2. **Hits can cite a section.**
3. **Index maintenance exists.** `remove`, `prune`, and multi-document
   `ingest` came with the change, because a design that needs a re-ingest
   needs a way to do one.

### Negative

1. **More vectors.** The measured vault went from 1,243 vectors to 28,185
   passages, and its database from 2.6 MB to 62 MB.
2. **Slower ingest.** A full ingest of that vault took about ten minutes on
   a laptop CPU. `ingest --changed-only` skips documents that have not
   changed.
3. **A re-ingest is required** to get the new ranking. Until then a
   document is ranked exactly as before.
4. **`find-similar` results change** for re-ingested documents, since the
   document vector is now the summary passage rather than the opening of
   the body.

### Neutral

1. **Brute force still holds.** One search over 28,185 passages took under
   0.1 seconds, model load included. ADR-0015's O(n) limit is about twenty
   times closer than it was and is still not near.
2. **The skipped sections are a convention.** `Relationships` and
   `Navigation` are link lists in the corpora this was built for. A corpus
   that uses those headings for prose would lose that prose from search.

## Decision Outcome

Measured on 2026-11-24 against the same vault (370 kit documents and 929
notes), before with `mif-cli` 0.11.0 and after with this change, the vault
fully re-ingested into a scratch database. Each query is a record's own
entity name (title) or its definition, or first statement; the figure is
how often the record itself is returned.

| Records | Query | Measure | Before | After |
| --- | --- | --- | --- | --- |
| Capability (116) | title | top 1 | 41% | 93% |
| Capability (116) | title | top 3 | 52% | 99% |
| Capability (115) | definition | top 1 | 20% | 94% |
| Capability (115) | definition | top 10 | 54% | 100% |
| Constraint (130) | title | top 1 | 29% | 76% |
| Constraint (130) | title | top 3 | 50% | 95% |
| Constraint (129) | definition | top 1 | 19% | 81% |
| Constraint (129) | definition | top 10 | 57% | 100% |

The targets set before the work were a definition query top 1 of 80% for
capability records and 70% for constraint records, a definition query top
10 of 95% for capability records, and a title query top 3 of 90% for both.
All are met. Option 3, a model change, was held in reserve for a miss and
was not needed.

The measurement is one corpus of one kind: long, templated, English
records. It says nothing about short notes, other languages, or corpora
without headings, where passages add little and the summary passage carries
the result.

## Related Decisions

- [ADR-0015: Local Embeddings and a SQLite Brute-Force Vector Store](https://modeled-information-format.github.io/mif-rs/adr/0015-local-embeddings-sqlite-vector-store/) — amended by this ADR.
- [ADR-0004: Libraries Never Depend on Binaries](https://modeled-information-format.github.io/mif-rs/adr/0004-libraries-never-depend-on-binaries/) — why the passage builder lives in `mif-embed` and both binaries call it.

## Links

- `crates/mif-embed/src/passages.rs` — the passage builder.
- `crates/mif-store/src/lib.rs` — "Documents and Passages" and "Schema Versions" in the module documentation.
- `crates/mif-cli/tests/retrieval.rs` — the regression test for this decision.

## More Information

The retrieval regression test builds five documents that share a boilerplate
opening longer than the model's window and differ only in one later
sentence, and requires each to be found by a paraphrase of its own
sentence. It fails on 0.11.0.

## Audit

### 2026-11-24

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Documents are split into a summary passage and heading-scoped passages of at most 256 tokens | crates/mif-embed/src/passages.rs | `passages`, `PASSAGE_MAX_TOKENS` | compliant |
| `Embedder::embed` and tokenizer truncation are unchanged | crates/mif-embed/src/lib.rs | `Embedder::load`, `Embedder::embed` | compliant |
| Search ranks by best passage; `top_k_similar` ranks document vectors | crates/mif-store/src/lib.rs | `search`, `top_k_similar` | compliant |
| Both binaries ingest through `Embedder::embed_document` | crates/mif-cli/src/main.rs, crates/mif-mcp/src/main.rs | `ingest_one`, `ingest_mif_document_inner` | compliant |

**Summary:** The implementation matches the decision. The measurements in
Decision Outcome were taken with the code this ADR describes.

**Action Required:** None.
