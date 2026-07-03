# mif-mcp

MCP server for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Exposes six tools over stdio, mirroring `mif-cli`:

- `validate_mif_document` — validate a MIF document against the canonical schema.
- `resolve_ontology_reference` — resolve an ontology's three-tier `extends` chain.
- `ingest_mif_document` — lint, validate, prove a lossless round trip, compute an embedding, and store the embedding vector for one MIF document (markdown with frontmatter, or JSON-LD). `db_path` defaults to `.mif/vectors.db`, created (with its parent directory) if absent.
- `search_documents` — free-text semantic search over previously ingested documents. `limit` defaults to 10.
- `find_similar_documents` — find previously ingested documents similar to an already-ingested one, identified by its id. `limit` defaults to 10 and excludes the anchor document itself.
- `corpus_stats` — summary statistics (count, embedding dimensionality) over the vector store.

## Error output format

An MCP client is inherently a machine consumer — there is no terminal to detect — so every tool failure renders as a compact RFC 9457 `application/problem+json` envelope rather than plain text, using the same `mif_problem`/`ToProblem` pattern as `mif-cli`'s `--format json`.

## License

MIT
