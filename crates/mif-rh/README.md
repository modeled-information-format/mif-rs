# mif-rh

Compiled ontology resolution/review engine for
[research-harness-template](https://github.com/modeled-information-format/research-harness-template)
(rht) corpora, in the [MIF (Modeled Information Format)](https://mif-spec.dev)
ecosystem.

Reimplements the observable behavior of rht's `scripts/resolve-ontology.sh`
and `scripts/ontology-review.sh` — classifying findings against topic-bound
domain ontologies, validating each finding's `entity` payload, and
aggregating per-topic coverage — as a single, self-contained library with no
`yq`/`jq`/`ajv` subprocess dependency. `resolve()`/`review()` are entirely
deterministic and rule-based; the read-only hypothesis layer —
`suggest_type()` (tier-annotated entity-type suggestions per MIF ADR-020's
confidence policy, built on `mif-ontology`'s `EntityType::embedding_doc()`
and `confidence` model) and the `FindingIndex` behind `find_similar` —
never influences the `ontology-map.json` classification itself.

## License

MIT
