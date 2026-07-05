# mif-rh-mcp

MCP server for [`mif-rh`](../mif-rh), the compiled ontology resolution/review
engine for
[research-harness-template](https://github.com/modeled-information-format/research-harness-template)
corpora.

Exposes `search`, `suggest_type`, `find_similar`, and `corpus_stats` as MCP
tools, read-only over the index `mif-rh-cli review --build-index` builds
(default path `reports/_meta/search-index.sqlite`). `suggest_type` returns a
ranked hypothesis and never writes to `reports/` — a human or agent confirms
it via rht's own `/ontology-review --enrich` step.

`suggest_type` results carry confidence-tier annotations (MIF ADR-020):
each candidate's `tier` is one of `auto_classify_eligible`,
`flag_for_review`, or `trigger_expansion` under the corpus's calibration
artifact (default `reports/_meta/confidence-calibration.json`; absent means
conservative built-in thresholds and `calibrated: false`). `suggest_type`
matches against each entity type's positive embedding document
(`description` + `aliases` + `exemplars`), and only a top candidate that
clears both the calibrated floor and a top-1/top-2 margin is
`auto_classify_eligible` — which is still a hypothesis, never an
auto-stamp. Types carrying curated `negative_examples` also pass through
the negative-demotion-v1 gate: a candidate whose query sits at least as
close to one of its negatives as to its positive document is barred from
tier 1 and carries `negative_demoted: true`; types without negatives score
exactly as before. `find_similar` carries a similarity band instead
(`near_duplicate`/`related`/`weak`, under the same calibrated floors) —
deliberately not the classification-tier vocabulary, because similarity
recall is not a classification decision.

## License

MIT
