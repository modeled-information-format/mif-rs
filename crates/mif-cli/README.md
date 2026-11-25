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
mif-cli prune --root . --dry-run --ignore 'archive/**'
mif-cli --format json search "P23" --entity-type practice
mif-cli search "a furry pet cat" --mode semantic
mif-cli get-passage urn:mif:9f2c4e1a-7b3d-4c8e-a1f0-5d6b7c8e9a0b --heading Scope
mif-cli --format json validate document.json
```

`validate` accepts markdown-with-frontmatter or a JSON-LD projection, proving the markdown <-> JSON-LD round trip is lossless either way, with no side effects (no embedding model load, no vector store write). `ingest` lints, validates, proves a lossless round trip, computes an embedding, and stores it (`--db-path` defaults to `.mif/vectors.db`). `search` and `find-similar` rank previously ingested documents by similarity (`--limit` defaults to 10). `corpus-stats` reports count and embedding dimensionality over the vector store. All three accept a repeatable `--extra-db-path` alongside `--db-path` to query multiple vector store roots (e.g. a project-local store layered with a shared central one): `search` and `find-similar` merge-rank matches from every root by cosine similarity into one result list; `corpus-stats` instead reports a summed `total_count` across every root followed by one line per root's own count/dim (there is no query vector in a stats call, so nothing is ranked). Omitting `--extra-db-path` queries only `--db-path` (or its default), unchanged from single-root usage.

`ingest` embeds each document as passages: a summary passage (title, entity name, description, and the entity definition or first statement), then one or more passages per body section. Its result line reports the count as `passages=`. Given several files or a directory (every `.md` file with frontmatter beneath it, skipping hidden directories), it loads the model once, attempts every document, prints one result line per document, reports each failure on stderr as one plain text line (`FILE: error: ...`, whatever `--format` says), and exits non-zero if any failed; `--changed-only` skips a document the store already holds with the same content. `search` ranks each document by its best-matching passage. `--id-prefix`, `--source-prefix` (the path recorded at ingest, relative to the directory that holds the store's own directory: for `vault/.mif/vectors.db`, `vault`), and `--namespace-prefix` limit which documents are ranked, and `--show-passage` appends the heading of the best-matching passage to each line as `[Heading]`; without it the output is `score  id`, as before. `find-similar` compares document vectors (the summary passage). `corpus-stats` for one root also reports `passages=` and `documents_without_passages=`, the documents not ingested since passages existed. `remove` deletes one document's vectors by id. `prune --root DIR` deletes every stored document whose id no file under `DIR` carries (`--dry-run` lists them); it stops without removing anything if no document is found under `DIR`, or if a file's frontmatter cannot be parsed.

`search` ranks in one of two ways. `--mode hybrid`, the default, fuses the semantic ranking with a lexical one over passage text, so an identifier (`P23`) or an exact term is found by the passage that holds it; the score printed is then a reciprocal rank fusion value (at most `2/61`), not a cosine similarity. `--mode semantic` ranks by cosine similarity alone and gives the order and scores of releases before 0.13.0. A document ingested before 0.13.0 has no stored text and is ranked by its vectors in either mode until it is ingested again; `corpus-stats` counts these as `documents_without_text=`. `--entity-type`, `--exclude-entity-type`, `--concept-type`, and `--exclude-concept-type` limit results by the document's `entity.entity_type` and `conceptType`; each takes one exact value and can be repeated.

With `--format json` given explicitly, `search` prints `{"matches": [...]}` instead of text lines. Each match has `id`, `score`, `cosine`, `bm25` (null when no passage holds a query term), `path`, `title`, `entity_type`, `concept_type`, `heading`, `text` (the matching passage's own text), and `span` (`start_char`, `end_char`, `start_line`, `end_line` in the document body: characters from 0 with the end exclusive, lines from 1), plus `root` for a multi-root search. Without the flag the output stays `score  id`, whatever the error format defaults to.

`get-passage ID --heading HEADING` prints that section of an ingested document as it was written, so a caller can read one section instead of the whole file. A heading that occurs more than once prints each occurrence; without `--heading` it prints the document's summary. It accepts `--extra-db-path` and takes the document from the first root that holds it, and prints JSON (`id`, `sections` each with `heading`, `text`, `span`) under `--format json`.

`ingest` and `prune` take a repeatable `--ignore GLOB`. A pattern with no `/` matches a file or directory name at any depth (`drafts`, `*.draft.md`); one with a `/` matches the path relative to the directory being walked (`notes/archive/**`). `*` and `?` stay within one path component and `**` crosses them; there is no negation. `prune` does not look in hidden directories, which `ingest` never ingests from, so a copy of a deleted document left in one (a trash folder, say) no longer keeps it in the store.

The global `--format pretty|json` flag selects how errors render (defaults to `pretty` on a terminal, `json` otherwise): `pretty` prints plain `Error: ...` text; `json` renders a compact RFC 9457 `application/problem+json` envelope on stderr.

## License

MIT
