# mif-mcp

MCP server for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Exposes nine tools over stdio, mirroring `mif-cli`:

- `validate_mif_document` — validate a MIF document (markdown with frontmatter, or JSON-LD) against the canonical schema, with no side effects (no embedding model load, no vector store write).
- `resolve_ontology_reference` — resolve an ontology's three-tier `extends` chain.
- `ingest_mif_document` — lint, validate, prove a lossless round trip, compute an embedding, and store the embedding vector for one MIF document (markdown with frontmatter, or JSON-LD). `db_path` defaults to `.mif/vectors.db`, created (with its parent directory) if absent.
- `search_documents` — free-text semantic search over previously ingested documents. `limit` defaults to 10.
- `find_similar_documents` — find previously ingested documents similar to an already-ingested one, identified by its id. `limit` defaults to 10 and excludes the anchor document itself.
- `corpus_stats` — summary statistics (count, embedding dimensionality) over the vector store.
- `roundtrip_mif_document` — prove a MIF document's markdown <-> JSON-LD round trip is lossless. Pure: no db, no embedder.
- `emit_jsonld_document` — project a MIF document to its canonical JSON-LD form, proving the round trip is lossless in the process. Pure: no db, no embedder.
- `emit_markdown_document` — project a JSON-LD MIF document to its canonical markdown-with-frontmatter form, proving the round trip is lossless in the process. Pure: no db, no embedder.

`search_documents`, `find_similar_documents`, and `corpus_stats` all also accept an `extra_db_paths` array alongside `db_path` to query multiple vector store roots (e.g. a project-local store layered with a shared central one): `search_documents` and `find_similar_documents` merge-rank matches from every root by cosine similarity into one result list; `corpus_stats` instead returns a summed `total_count` across every root plus a per-root `extra_roots` breakdown (there is no query vector in a stats call, so nothing is ranked). Omitting `extra_db_paths` (or passing an empty array) queries only `db_path` (or its default), unchanged from single-root usage — the JSON output shape is also unchanged in that case.

## Error output format

An MCP client is inherently a machine consumer — there is no terminal to detect — so every tool failure renders as a compact RFC 9457 `application/problem+json` envelope rather than plain text, using the same `mif_problem`/`ToProblem` pattern as `mif-cli`'s `--format json`.

## License

MIT
