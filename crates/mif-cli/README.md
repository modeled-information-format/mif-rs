# mif-cli

Command-line interface for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

```bash
mif-cli validate document.md
mif-cli validate document.json
mif-cli ontology resolve grazing-plan --ontologies-dir ./ontologies
mif-cli ingest document.md --db-path .mif/vectors.db
mif-cli search "a furry pet cat" --limit 5
mif-cli find-similar urn:mif:9f2c4e1a-7b3d-4c8e-a1f0-5d6b7c8e9a0b --limit 5
mif-cli corpus-stats --db-path .mif/vectors.db
mif-cli search "a furry pet cat" --extra-db-path ~/.mif/central-vectors.db
mif-cli ingest docs/ notes/extra.md --changed-only
mif-cli search "a furry pet cat" --source-prefix docs/ --show-passage
mif-cli remove urn:mif:9f2c4e1a-7b3d-4c8e-a1f0-5d6b7c8e9a0b
mif-cli prune --root . --dry-run
mif-cli --format json validate document.json
```

`validate` accepts markdown-with-frontmatter or a JSON-LD projection, proving the markdown <-> JSON-LD round trip is lossless either way, with no side effects (no embedding model load, no vector store write). `ingest` lints, validates, proves a lossless round trip, computes an embedding, and stores it (`--db-path` defaults to `.mif/vectors.db`). `search` and `find-similar` rank previously ingested documents by similarity (`--limit` defaults to 10). `corpus-stats` reports count and embedding dimensionality over the vector store. All three accept a repeatable `--extra-db-path` alongside `--db-path` to query multiple vector store roots (e.g. a project-local store layered with a shared central one): `search` and `find-similar` merge-rank matches from every root by cosine similarity into one result list; `corpus-stats` instead reports a summed `total_count` across every root followed by one line per root's own count/dim (there is no query vector in a stats call, so nothing is ranked). Omitting `--extra-db-path` queries only `--db-path` (or its default), unchanged from single-root usage.

`ingest` embeds each document as passages: a summary passage (title, entity name, description, and the entity definition or first statement), then one or more passages per body section. Its result line reports the count as `passages=`. Given several files or a directory (every `.md` file with frontmatter beneath it, skipping hidden directories), it loads the model once, attempts every document, prints one result line per document, reports each failure on stderr, and exits non-zero if any failed; `--changed-only` skips a document the store already holds with the same content. `search` ranks each document by its best-matching passage. `--id-prefix`, `--source-prefix` (the path recorded at ingest, relative to the directory `ingest` ran in), and `--namespace-prefix` limit which documents are ranked, and `--show-passage` appends the heading of the best-matching passage to each line as `[Heading]`; without it the output is `score  id`, as before. `find-similar` compares document vectors (the summary passage). `corpus-stats` for one root also reports `passages=` and `documents_without_passages=`, the documents not ingested since passages existed. `remove` deletes one document's vectors by id. `prune --root DIR` deletes every stored document whose id no file under `DIR` carries (`--dry-run` lists them); it stops without removing anything if no document is found under `DIR`, or if a file's frontmatter cannot be parsed.

The global `--format pretty|json` flag selects how errors render (defaults to `pretty` on a terminal, `json` otherwise): `pretty` prints plain `Error: ...` text; `json` renders a compact RFC 9457 `application/problem+json` envelope on stderr.

## License

MIT
