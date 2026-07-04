# mif-ontology

Ontology resolution and classification-confidence capabilities for the
[MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Resolves the three-tier ontology inheritance chain (`mif-base` ->
`shared-traits` -> domain ontologies) driven by each ontology definition's
own `extends` list. Ontology content itself is not vendored — load a corpus
from a local directory of ontology definition YAML files (e.g. a checkout of
the `ontologies` repository) via `load_corpus_from_dir`, then resolve with
`resolve_chain`.

Also carries the MIF-level model for embedding-based entity-type
classification (MIF ADR-020): `EntityType` models the full
`ontology.schema.json` entity-type shape including the v1.1 classification
fields (`aliases`, `exemplars`, `negative_examples`) and composes each
type's positive embedding document (`EntityType::embedding_doc()` —
description + aliases + exemplars, never negative examples); the
`confidence` module defines the two-threshold, three-tier score policy
(`auto_classify_eligible`/`flag_for_review`/`trigger_expansion`, with a
top-1/top-2 margin gate on the top tier), the recalibratable
`CalibrationConfig` artifact, and the mutual-similarity cluster criterion
for expansion candidates. All pure model code — consumers supply the
embedding inference and persistence.

`OntologyError` implements `mif_problem::ToProblem`, mapping its seven
variants — `Io`, `Yaml`, `Invalid`, `Deserialize`, `NotFound`, `Cycle`,
and `CalibrationInvalid` — each to its own distinct RFC 9457
`application/problem+json` representation.

## License

MIT
