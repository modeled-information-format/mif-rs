# mif-ontology

Ontology resolution for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Resolves the three-tier ontology inheritance chain (`mif-base` ->
`shared-traits` -> domain ontologies) driven by each ontology definition's
own `extends` list. Ontology content itself is not vendored — load a corpus
from a local directory of ontology definition YAML files (e.g. a checkout of
the `ontologies` repository) via `load_corpus_from_dir`, then resolve with
`resolve_chain`.

`OntologyError` implements `mif_problem::ToProblem`, mapping its six
variants — `Io`, `Yaml`, `Invalid`, `Deserialize`, `NotFound`, and
`Cycle` — each to its own distinct RFC 9457 `application/problem+json`
representation.

## License

MIT
