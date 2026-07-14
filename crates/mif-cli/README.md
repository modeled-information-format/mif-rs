# mif-cli

Command-line interface for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

```bash
mif-cli validate document.md
mif-cli validate document.json
mif-cli ontology resolve grazing-plan --ontologies-dir ./ontologies
mif-cli ingest document.md --db-path .mif/vectors.db
mif-cli search "a furry pet cat" --limit 5
mif-cli find-similar urn:mif:memory:cats --limit 5
mif-cli corpus-stats --db-path .mif/vectors.db
mif-cli search "a furry pet cat" --extra-db-path ~/.mif/central-vectors.db
mif-cli --format json validate document.json
```

`validate` accepts markdown-with-frontmatter or a JSON-LD projection, proving the markdown <-> JSON-LD round trip is lossless either way, with no side effects (no embedding model load, no vector store write). `ingest` lints, validates, proves a lossless round trip, computes an embedding, and stores it (`--db-path` defaults to `.mif/vectors.db`). `search` and `find-similar` rank previously ingested documents by similarity (`--limit` defaults to 10). `corpus-stats` reports count and embedding dimensionality over the vector store. All three accept a repeatable `--extra-db-path` alongside `--db-path` to query multiple vector store roots (e.g. a project-local store layered with a shared central one): `search` and `find-similar` merge-rank matches from every root by cosine similarity into one result list; `corpus-stats` instead reports a summed `total_count` across every root followed by one line per root's own count/dim (there is no query vector in a stats call, so nothing is ranked). Omitting `--extra-db-path` queries only `--db-path` (or its default), unchanged from single-root usage.

The global `--format pretty|json` flag selects how errors render (defaults to `pretty` on a terminal, `json` otherwise): `pretty` prints plain `Error: ...` text; `json` renders a compact RFC 9457 `application/problem+json` envelope on stderr.

## License

MIT
