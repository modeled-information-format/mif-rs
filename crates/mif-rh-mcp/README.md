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

## License

MIT
