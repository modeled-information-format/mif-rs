# mif-store

SQLite vector store for document embeddings in the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

`VectorStore` wraps a single SQLite connection (`rusqlite`, bundled) and a
single `embeddings` table (`id`, `dim`, `vector`, `content_hash`,
`updated_at`). Vectors are stored as a raw little-endian `f32` blob — a
private, on-disk format read and written only by this crate, not a public
interchange format. This crate does not compute embeddings or timestamps;
both are supplied by the caller, keeping the store itself deterministic and
easy to test.

`VectorStore::open` opens (or creates) the database at a path and
initializes the schema if needed; the database file itself is created by
SQLite if absent, but a missing parent directory is reported as an error
rather than silently created. `upsert` inserts a new embedding or replaces
the existing one for an `id`; `get` looks one up; `count` and `stats`
report the store's size and (one representative) dimensionality;
`top_k_similar` ranks every stored embedding against a query vector by
cosine similarity and returns the top matches in descending score order.
`StoredVector` is what `get` returns, `SimilarityMatch` is one ranked
result from `top_k_similar`, and `CorpusStats` is the summary `stats`
returns.

Similarity search is brute-force: `top_k_similar` decodes and scores every
row in the store rather than consulting an approximate-nearest-neighbor
index. This is a deliberate choice, not an oversight — at the corpus sizes
this crate targets (a few thousand rows), brute-force cosine is simpler
and fast enough that an ANN index isn't warranted. Rows whose `dim` differs
from the query's are skipped (an incomparable embedding space, e.g. after
a model change); rows whose decoded vector length doesn't match their own
`dim` column are treated as data corruption and abort the query with
`StoreError::VectorBlobMismatch`, matching `get`'s behavior, rather than
silently skipping them.

Beside its document vector, a document can have passage vectors, one per
passage it was split into. `upsert_document` takes a `DocumentVectors` (the
id, the document vector, a slice of `PassageVector`s each with the heading
it came from, the content hash, the timestamp, and an optional source path
and namespace) and replaces everything stored for that id in one
transaction. `search` ranks each document once, by the cosine similarity of
its best-matching passage, and returns `SearchMatch`es carrying that
passage's heading; a document with no passages is ranked by its document
vector. A `SearchFilter` limits `search` to documents whose id, source, or
namespace starts with a given prefix. `top_k_similar` keeps ranking
document vectors, which is what document-to-document similarity uses.
`remove` deletes one document with its passages, `prune` deletes every
document whose id is not in a keep set and returns the removed ids, and
`passage_stats` returns a `PassageStats`: the passage count and the number
of documents that have none. `ids` lists every stored id, and `is_current`
says whether a document is already stored with a given content hash, the
same source, and current passages, so an ingest can skip it.
`document_source(db_path, file)` gives the source to record for a file: its
path relative to the directory that holds the store's own directory, so the
same document gets the same source wherever the ingesting process runs.

A passage can be stored with its own text and the span it covers in the
document body (`PassageVector::text`, `span`), and a document with its
title, entity type, concept type, and body sections as written
(`DocumentVectors::title`, `entity_type`, `concept_type`, `sections`).
`query` takes a `SearchQuery` and ranks by its `SearchMode`. `Semantic` is
the cosine ranking `search` gives. `Hybrid`, the default, also ranks
documents by the FTS5 `bm25` of their best passage text, and orders them by
reciprocal rank fusion: `1 / (60 + rank)` summed over a document's rank in
each ranking it appears in, so `SearchMatch::score` is then a fusion value
and `SearchMatch::cosine` the similarity. A document stored without text
takes part in the cosine ranking only; `PassageStats::documents_without_text`
counts them. Each `SearchMatch` carries the passage's heading, text, and
span and the document's source, title, and types, and `SearchFilter` has
exact-match include and exclude lists for entity type and concept type.
`sections(id, heading)` returns the stored sections with that heading, one
per occurrence, or with no heading the summary passage's text.
`multi_root_query` and `multi_root_sections` do the same across roots:
cosine similarities are ranked together, and each document keeps the
lexical rank it has in its own root.

The schema version is recorded in `PRAGMA user_version`: 0 is the
single-table layout of releases up to 0.11.0, and 1 adds the `passages`
table (`doc_id`, `ord`, `heading`, `dim`, `vector`, `content_hash`) and the `source` and
`namespace` columns on `embeddings`. `open` upgrades a version-0 database in
place, in one transaction, without rewriting any row; a version-0
database that is read-only is opened as it is and searched on its document
vectors. Each passage row carries the content hash of the document it was
computed from, and `search` uses only passages whose hash matches the
document row, so passages left behind by a release that predates them are
ignored.

Version 2 adds `title`, `entity_type`, and `concept_type` on `embeddings`;
`section`, `text_rowid`, and the four span columns on `passages`; the
`sections` table; and `passage_text`, an FTS5 table holding passage text.
`open` upgrades a version-1 database the same way, and a read-only one is
opened as it is and ranked by its vectors. Releases 0.11.0 and 0.12.0 can
still read and write an upgraded database: a trigger deletes a passage's
text when its row is deleted, and another deletes a document's sections
when its row is deleted. A document an older release stores again has no
text until this release ingests it.

`StoreError` implements `mif_problem::ToProblem` for RFC 9457
`application/problem+json` reporting.

A single `VectorStore` is always rooted at one database file. Querying
across several roots at once (e.g. a project-local store layered with a
shared central one) is not a method on `VectorStore` itself — it is the
free functions `multi_root_top_k_similar`, `multi_root_search`,
`multi_root_get`, and `multi_root_stats`, which each open every given root
independently and merge the results, failing closed on the first root that
cannot be opened or queried. `RootedMatch`, `RootedSearchMatch`, and
`MultiRootStats` are their result types, carrying which root each result
came from.

## License

MIT
