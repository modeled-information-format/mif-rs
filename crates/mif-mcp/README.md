# mif-mcp

MCP server for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Exposes eleven tools over stdio, mirroring `mif-cli`:

- `validate_mif_document` — validate a MIF document (markdown with frontmatter, or JSON-LD) against the canonical schema, with no side effects (no embedding model load, no vector store write).
- `resolve_ontology_reference` — resolve an ontology's three-tier `extends` chain.
- `ingest_mif_document` — lint, validate, prove a lossless round trip, embed, and store one MIF document (markdown with frontmatter, or JSON-LD). The document is embedded as a summary passage plus one or more passages per body section; the result's `passages` is the count. `db_path` defaults to `.mif/vectors.db`, created (with its parent directory) if absent.
- `search_documents` — free-text search over previously ingested documents. `mode` is `hybrid` by default: each document is ranked by reciprocal rank fusion of its best-matching passage by meaning and its best by exact terms, so an identifier such as `P23` is found by the text that holds it, and `score` is a fusion value (at most `2/61`) while `cosine` is the similarity. `mode: "semantic"` ranks by cosine similarity alone, as releases before 0.13.0 did. Each match carries `cosine`, `bm25` (when a passage holds a query term), the document's `path`, `title`, `entity_type`, and `concept_type`, and the matching passage's `heading`, `text`, and `span` (character offsets and lines in the document body); a field with nothing to report is omitted. `id_prefix`, `source_prefix`, and `namespace_prefix`, and the exact-match lists `entity_types`, `exclude_entity_types`, `concept_types`, and `exclude_concept_types`, limit which documents are ranked. `limit` defaults to 10.
- `get_passage` — fetch one section of an ingested document as it was written, by `id` and the `heading` a match reports, instead of reading the whole document. A heading that occurs more than once returns each occurrence; without `heading` it returns the document's summary.
- `remove_document` — remove one document's vectors from the vector store by id; `removed` says whether anything was stored for it.
- `find_similar_documents` — find previously ingested documents similar to an already-ingested one, identified by its id. `limit` defaults to 10 and excludes the anchor document itself.
- `corpus_stats` — summary statistics (count, embedding dimensionality) over the vector store. A single-root query also returns `passages`, `documents_without_passages`, and `documents_without_text` (documents `hybrid` search ranks by their vectors alone until they are ingested again).
- `roundtrip_mif_document` — prove a MIF document's markdown <-> JSON-LD round trip is lossless. Pure: no db, no embedder.
- `emit_jsonld_document` — project a MIF document to its canonical JSON-LD form, proving the round trip is lossless in the process. Pure: no db, no embedder.
- `emit_markdown_document` — project a JSON-LD MIF document to its canonical markdown-with-frontmatter form, proving the round trip is lossless in the process. Pure: no db, no embedder.

`search_documents`, `find_similar_documents`, `get_passage`, and `corpus_stats` all also accept an `extra_db_paths` array alongside `db_path` to query multiple vector store roots (e.g. a project-local store layered with a shared central one): `search_documents` and `find_similar_documents` merge-rank matches from every root into one result list (in `hybrid` mode, cosine similarities are ranked together and each document keeps the lexical rank it has in its own root); `get_passage` takes the document from the first root that holds it; `corpus_stats` instead returns a summed `total_count` across every root plus a per-root `extra_roots` breakdown (there is no query vector in a stats call, so nothing is ranked). Omitting `extra_db_paths` (or passing an empty array) queries only `db_path` (or its default), unchanged from single-root usage, and no `root` field appears in the output.

## Error output format

An MCP client is inherently a machine consumer — there is no terminal to detect — so every tool failure renders as a compact RFC 9457 `application/problem+json` envelope rather than plain text, using the same `mif_problem`/`ToProblem` pattern as `mif-cli`'s `--format json`.

## License

MIT
