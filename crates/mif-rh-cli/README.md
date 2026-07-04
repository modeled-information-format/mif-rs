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

`suggest-type <TEXT> --topic <T>` (or `suggest-type --finding <path>`)
prints a JSON array of entity-type hypotheses ranked by embedding
similarity, each annotated with a confidence tier
(`auto_classify_eligible`/`flag_for_review`/`trigger_expansion`, MIF
ADR-020) under the corpus's calibration artifact
(`reports/_meta/confidence-calibration.json` by default; absent means
built-in thresholds and `calibrated: false`). Hypotheses only — it never
writes a finding's `entity_type`. With `--record` (requires `--finding`),
a tier-3 miss is persisted in the index for `expansion-candidates`.

The full ADR-020 tier routing:

- `review --suggest` writes tier-annotated suggestion queues to
  `<reports-dir>/_meta/suggestions/<topic>.json` for this review's
  not-durably-stamped findings (confirmed/rejected verdicts from
  `/ontology-review --enrich` are preserved on re-runs), and records
  tier-3 misses.
- `calibrate` derives the corpus's calibration artifact from its stamped
  findings (`stamped-quantile-v1`: loosest floor+margin gate meeting
  `--target-precision`, tier-2 floor from gold-recall quantiles).
- `expansion-candidates` clusters recorded misses (mutual similarity,
  minimum cluster size, minimum distinct runs) into ontology-expansion
  candidates, as JSON for `author-ontology.sh --from-clusters`.

`--relationship-script` is Unix-only: it spawns the given script directly
and relies on its `#!` shebang, which Windows does not honor. Leave it
unset on Windows (the default auto-detection already no-ops when the
script isn't found) or run under a POSIX-compatible shell.

## License

MIT
