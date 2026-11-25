---
title: "Hybrid Search and Stored Passage Text"
description: "Store each passage's text and each section's markdown beside the vectors, rank search by reciprocal rank fusion of cosine similarity and bm25 by default, and return the matching passage with its document, amending ADR-0021's vectors-only store and cosine-only ranking."
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
  - full-text-search
  - architecture
status: accepted
created: 2026-11-25
updated: 2026-11-25
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0021-passage-level-embedding-and-search.md
  - 0015-local-embeddings-sqlite-vector-store.md
  - 0004-libraries-never-depend-on-binaries.md
---

# ADR-0022: Hybrid Search and Stored Passage Text

## Status

Accepted. Amends [ADR-0021: Passage-Level Embedding and Best-Passage
Search](https://modeled-information-format.github.io/mif-rs/adr/0021-passage-level-embedding-and-search/):
the passage builder, the passage vectors, and best-passage cosine ranking
all stand, and remain available unchanged as `--mode semantic`. What changes
is that the store also keeps text, that `search` ranks by a fusion of two
rankings by default, and what a match carries.

## Context

### Background and Problem Statement

Release 0.12.0 ranks each document by the cosine similarity of its
best-matching passage. That finds a document by what its text means. It
does not find one by an identifier or an exact term. A query such as `P23`
or `multi_root_search` has no meaning for the model to place, so the
passage that contains it scores no better than its neighbours.

An agent using the search also had to read a whole document to use a hit.
A match carried a document id, a score, and a heading. It did not carry the
text that matched, where that text sits, or what kind of document it came
from, and there was no way to fetch one section.

Measured on 2026-11-25 with `mif-cli` 0.12.0 over 191 documents (1,836
passages) taken from the `gdlc`, `mif-docs-plugin`, and `MIF` repositories:
for 60 queries that are each an identifier occurring in exactly one
document, that document was returned first for 22% and in the top ten for
57%.

### Current Limitations

1. **Exact terms rank poorly.** Cosine similarity over sentence embeddings
   is the only signal.
2. **A hit does not carry its text.** The store holds vectors and headings
   only, so nothing can be returned or indexed.
3. **Results cannot be limited by kind.** The store records no entity type
   or concept type.
4. **A stale copy keeps a document alive.** `prune` reads hidden
   directories that `ingest` never ingests from, so a deleted note with a
   copy in a trash folder is never pruned.

## Decision Drivers

### Primary Decision Drivers

1. **An identifier or an exact term must find the passage that holds it.**
2. **Queries by meaning must keep working.** The ranking of 0.12.0 must
   stay available exactly, since callers and one downstream skill compare
   its scores with a threshold.
3. **A match must be usable without reading the document**: its text, its
   place in the body, and a way to fetch its section.
4. **An older binary must still be able to use an upgraded store.**
   ADR-0021 promised this for 0.11.0, and editors and scripts run mixed
   versions against one database file.

### Secondary Decision Drivers

1. **The offline, single-binary design of ADR-0015 stays.** No new service
   and no new index format beside `SQLite`.
2. **No tuning knob in the default.** A ranking that needs a weight chosen
   per corpus will be wrong on most corpora.

## Considered Options

### Option 1: Keep the store vectors-only and read text from the source files

**Description**: Leave the database as it is. At query time, re-read each
matching document from its recorded source path and rebuild its passages to
get text and spans.

**Advantages**:

- No schema change, and no document content in the database.

**Disadvantages**:

- There is nothing to run a lexical ranking on: `bm25` needs an index over
  every passage, not the text of the documents that already matched.
- A result breaks when a file has moved or changed since ingest.

**Risk Assessment**:

- **Technical Risk**: Medium. Text and vectors can disagree silently.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low. But it does not meet the first driver at all.

### Option 2: Store passage text with an FTS5 index, fuse rankings by reciprocal rank fusion (chosen)

**Description**: Store each passage's own text in an `SQLite` FTS5 table
and each section's markdown in a `sections` table. Rank documents twice,
by best-passage cosine similarity and by best-passage `bm25`, and order
them by the sum of `1 / (60 + rank)` over the rankings each appears in.

**Advantages**:

- Exact terms are found, and a query by meaning still has its ranking.
- Reciprocal rank fusion needs no weight and no score normalization.
- FTS5 is already compiled into the bundled `SQLite` (and is already used
  by `mif-rh`), so there is no new dependency for search.
- A match can carry its text and span, and a section can be fetched alone.

**Disadvantages**:

- The database holds document content and grows by about half again.
- A re-ingest is needed before a document has text.
- The default score is no longer a cosine similarity.

**Risk Assessment**:

- **Technical Risk**: Low. Both rankings are `SQLite` queries and a sort.
- **Schedule Risk**: Medium. It touches the passage builder, the store
  schema, and both binaries.
- **Ecosystem Risk**: Medium. Result order and the meaning of `score`
  change for every caller; see Consequences.

### Option 3: A weighted blend of normalized scores

**Description**: Store text as in Option 2, normalize `bm25` to a 0 to 1
range, and rank by a weighted sum with cosine similarity.

**Advantages**:

- The score stays on a familiar scale.

**Disadvantages**:

- The weight and the normalization both have to be chosen, and the right
  values differ by corpus.
- `bm25` values are not comparable between two stores, so a search across
  several roots has no sound way to blend them.

**Risk Assessment**:

- **Technical Risk**: Medium.
- **Schedule Risk**: Medium.
- **Ecosystem Risk**: Medium, as for Option 2.

### Option 4: Hybrid ranking as an opt-in, semantic by default

**Description**: Option 2, with `--mode semantic` as the default.

**Advantages**:

- Nothing changes for an existing caller.

**Disadvantages**:

- An agent searching for an identifier has to know to ask for the other
  mode, and the query that needs hybrid ranking most looks like any other.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

## Decision

We store **passage text** and **section markdown** beside the vectors, and
`search` ranks by **reciprocal rank fusion** of a semantic and a lexical
ranking **by default**.

- `mif_embed::passages` gives each passage its own text (without the title
  and heading line, and without the words carried from the passage before
  it) and the span of body lines that text came from. `sections` returns
  each body section as written. Spans are positions in the `content` string
  of the document's JSON-LD projection: character offsets from 0, end
  exclusive, counted in Unicode scalar values, and lines from 1.
- `mif-store` schema version 2 adds `title`, `entity_type`, and
  `concept_type` to `embeddings`; a text pointer, span, and section index
  to `passages`; a `sections` table; and `passage_text`, an FTS5 table
  (`unicode61` tokenizer) holding passage text.
- `VectorStore::query` ranks documents. Under `SearchMode::Semantic` it is
  the ranking of ADR-0021 and the score is the cosine similarity. Under
  `SearchMode::Hybrid` each document also gets a lexical rank from the
  `bm25` of its best passage, and the score is `1 / (60 + r)` summed over
  its cosine rank and, when it has one, its lexical rank.
- Fusion is per document. The passage reported for a document is the one
  from whichever ranking places the document higher.
- Documents with the same `bm25` share a lexical rank. Templated documents
  hold the same boilerplate word for word, and a common query word matches
  it equally in all of them; an order made up among equals must not move
  one of them past a better semantic match.
- Each query word that holds a letter or digit becomes a quoted phrase, and
  any may match. Query text is never read as FTS5 syntax, and a hyphenated
  identifier matches as the adjacent tokens it is indexed as.
- Across several roots, cosine similarities are ranked together. Each
  document keeps the lexical rank it has in its own root, because a `bm25`
  value depends on the store it was computed in.
- A match carries the document's source path, title, and types, the
  passage's heading, text, and span, and `cosine` and `bm25` beside
  `score`. `SearchFilter` gains exact-match include and exclude lists for
  entity type and concept type.
- `VectorStore::sections` returns the sections of a document with a given
  heading as they were written, or the summary. `mif-cli get-passage` and
  the `get_passage` MCP tool expose it.
- `mif-cli prune` no longer reads hidden directories, and `ingest` and
  `prune` take `--ignore` globs.

Compatibility is part of the decision:

- A version-1 database is upgraded in place on open with no row rewritten.
  A read-only database is opened as it is and ranked by its vectors.
- Releases 0.11.0 and 0.12.0 can still read and write an upgraded
  database. Two triggers keep it consistent when they do: deleting a
  passage row deletes its text, and deleting a document row deletes its
  sections. A document re-ingested by an older release has no text until
  this release ingests it again.
- A document with no stored text takes part in the semantic ranking only.
  `corpus-stats` reports how many remain as `documents_without_text`.
- `--mode semantic` returns the order and scores of 0.12.0.
- Without `--format json`, `mif-cli search` prints `score  id` lines as
  before.

This amends three statements of ADR-0021. Its fourth primary driver,
"existing callers keep working", now holds for a caller that passes
`--mode semantic` and not for one that relies on the default order or on
`score` being a cosine similarity. Its statement that `search` prints
`score  id` "as before" still describes the format, but the default score
is a fusion value. Its statement that passage vectors are internal to the
store extends to passage text and sections: neither is part of the MIF
specification, and a document's `EmbeddingReference` is unaffected.

## Consequences

### Positive

1. **Identifiers and exact terms are found.** See Decision Outcome.
2. **An agent can use a hit without reading the document**, and can fetch
   one section as written.
3. **Results can be limited by kind.**
4. **A copy in a hidden or ignored directory no longer keeps a document.**

### Negative

1. **The default order and the meaning of `score` change.** A caller that
   compares `score` with a cosine threshold must read `cosine` or pass
   `--mode semantic`. The `mif-docs` plugin's `doc-set-planner` skill does
   this with a threshold of about 0.55 and needs a matching change.
2. **Queries by meaning can rank slightly worse.** A query that shares only
   common words with its target gains nothing from the lexical ranking and
   can lose a place to a document that matches those words. See Decision
   Outcome.
3. **The database grows and holds document content.** The measured store
   went from 4.3 MB to 6.8 MB.
4. **A re-ingest is required** for text, types, and sections.
5. **`prune` removes more.** A document whose only file is in a hidden or
   ignored directory is now pruned.

### Neutral

1. **`find-similar` is unchanged.** It compares document vectors.
2. **Skipped sections are not stored.** The `Relationships` and
   `Navigation` sections that ADR-0021 leaves out of the passages are also
   left out of `sections`, so `get_passage` cannot return them.
3. **A paragraph too long for one passage is reported whole.** Each passage
   it is cut into carries the span of the whole paragraph.
4. **`--ignore` is glob matching, not `.gitignore` semantics.** There is no
   negation.

## Decision Outcome

Measured on 2026-11-25 over the 191 documents described in Context,
ingested into a scratch database, with the release build of this change.
The semantic column was confirmed identical, rank for rank, to `mif-cli`
0.12.0 on all 141 queries. MRR is the mean reciprocal rank of the target
document.

| Query set | Queries | Measure | Semantic | Hybrid |
| --- | --- | --- | --- | --- |
| An identifier occurring in one document | 60 | top 1 | 22% | 87% |
| An identifier occurring in one document | 60 | top 10 | 57% | 100% |
| An identifier occurring in one document | 60 | MRR | 0.332 | 0.920 |
| A document's own title | 60 | top 1 | 93% | 97% |
| A document's own title | 60 | MRR | 0.961 | 0.981 |
| A paraphrase sharing no key term | 21 | top 1 | 48% | 43% |
| A paraphrase sharing no key term | 21 | top 10 | 81% | 86% |
| A paraphrase sharing no key term | 21 | MRR | 0.604 | 0.547 |

Hybrid ranking was better than semantic on 44 of the 60 identifier queries
and worse on none. On the paraphrase queries it was better on 5 and worse
on 6.

Four variations were measured against the same queries and rejected:
halving the weight of the lexical ranking, counting only the top ten
lexical matches, dropping common words from the lexical query, and using
only query terms that occur in few passages. Each either did no better on
the paraphrase queries or gave up most of the gain on identifiers.

Hybrid is the default because the gain on exact terms is large and
consistent and the loss on paraphrases is small, and because
`--mode semantic` restores the earlier ranking exactly.

The measurement has limits. It is one English documentation corpus. The
identifier and title queries were generated from the corpus, and the 21
paraphrase queries were written by hand for this measurement, so that row
is weak evidence in either direction. It was not repeated on the vault
that ADR-0021 measured.

## Related Decisions

- [ADR-0021: Passage-Level Embedding and Best-Passage Search](https://modeled-information-format.github.io/mif-rs/adr/0021-passage-level-embedding-and-search/) — amended by this ADR.
- [ADR-0015: Local Embeddings and a SQLite Brute-Force Vector Store](https://modeled-information-format.github.io/mif-rs/adr/0015-local-embeddings-sqlite-vector-store/) — the store this extends; cosine ranking stays brute-force.
- [ADR-0004: Libraries Never Depend on Binaries](https://modeled-information-format.github.io/mif-rs/adr/0004-libraries-never-depend-on-binaries/) — why fusion lives in `mif-store` and both binaries call it.

## Links

- `crates/mif-store/src/hybrid.rs` — ranking, fusion, and `sections`.
- `crates/mif-store/src/lib.rs` — "Schema Versions" in the module documentation.
- `crates/mif-embed/src/passages.rs` — spans and `sections`.
- `crates/mif-cli/tests/hybrid.rs` — the regression test for this decision.

## More Information

The regression test ingests three documents and requires a search for an
identifier to return the document that holds it first, with the passage's
text and a span that slices back to the source line. It also covers the
type filters, `--mode semantic`, and `get-passage`.

ADR-0021's regression test (`crates/mif-cli/tests/retrieval.rs`) runs
unchanged under the hybrid default and passes. It failed before lexical
ties shared a rank: its five documents open with identical boilerplate, a
paraphrase query shared the word "and" with that boilerplate, and the
arbitrary order of five equal lexical matches put the wrong document first.

## Audit

### 2026-11-25

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Passages carry their own text and a body span; sections are returned as written | crates/mif-embed/src/passages.rs | `Passage`, `Span`, `sections` | compliant |
| Schema version 2 adds text, sections, and types, with triggers for older writers | crates/mif-store/src/lib.rs | `MIGRATE_V2_SQL`, `migrate` | compliant |
| Hybrid ranking is per-document reciprocal rank fusion with constant 60 | crates/mif-store/src/hybrid.rs | `fuse`, `RRF_K` | compliant |
| Documents with equal `bm25` share a lexical rank | crates/mif-store/src/hybrid.rs | `rank_lexical` | compliant |
| Semantic mode returns the cosine ranking of ADR-0021 | crates/mif-store/src/hybrid.rs | `VectorStore::query`, `fuse` | compliant |
| Both binaries default to hybrid and expose `get_passage` | crates/mif-cli/src/main.rs, crates/mif-mcp/src/main.rs | `search`, `get_passage`, `search_documents`, `get_passage_inner` | compliant |

**Summary:** The implementation matches the decision. The measurements in
Decision Outcome were taken with the code this ADR describes.

**Action Required:** None.
