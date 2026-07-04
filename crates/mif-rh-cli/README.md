# mif-rh-cli

Command-line interface for [`mif-rh`](../mif-rh), the compiled ontology
resolution/review engine for
[research-harness-template](https://github.com/modeled-information-format/research-harness-template)
corpora.

Drop-in replacement for rht's `scripts/resolve-ontology.sh`
(`mif-rh-cli resolve`) and `scripts/ontology-review.sh`
(`mif-rh-cli review`) — same flags, same exit codes, same
`TOPIC BOUND FIND STAMPED DISCOVERY UNTYPED INVALID` table/summary output,
same `ontology-map.json`/`--followup` backlog output.

`review` also acquires an exclusive lock (`<reports-dir>/_meta/.review.lock`)
for the duration of the run, and — with `--build-index` — rebuilds the
corpus-wide search index (`<reports-dir>/_meta/search-index.sqlite`) that
`mif-rh-mcp`'s `search`/`find_similar` tools read.

## License

MIT
