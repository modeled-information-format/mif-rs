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

`StoreError` implements `mif_problem::ToProblem` for RFC 9457
`application/problem+json` reporting.

A single `VectorStore` is always rooted at one database file. Querying
across several roots at once (e.g. a project-local store layered with a
shared central one) is not a method on `VectorStore` itself — it is the
free functions `multi_root_top_k_similar`, `multi_root_get`, and
`multi_root_stats`, which each open every given root independently and
merge the results, failing closed on the first root that cannot be opened
or queried. `RootedMatch` and `MultiRootStats` are their result types,
carrying which root each result came from.

## License

MIT
