# mif-cli

Command-line interface for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

```bash
mif-cli validate document.json
mif-cli ontology resolve grazing-plan --ontologies-dir ./ontologies
mif-cli ingest document.md --db-path .mif/vectors.db
mif-cli search "a furry pet cat" --limit 5
mif-cli find-similar urn:mif:memory:cats --limit 5
mif-cli corpus-stats --db-path .mif/vectors.db
mif-cli --format json validate document.json
```

`ingest` lints, validates, proves a lossless round trip, computes an embedding, and stores it (`--db-path` defaults to `.mif/vectors.db`). `search` and `find-similar` rank previously ingested documents by similarity (`--limit` defaults to 10). `corpus-stats` reports count and embedding dimensionality over the vector store.

The global `--format pretty|json` flag selects how errors render (defaults to `pretty` on a terminal, `json` otherwise): `pretty` prints plain `Error: ...` text; `json` renders a compact RFC 9457 `application/problem+json` envelope on stderr.

## License

MIT
