# mif-core

Shared foundational types for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Provides `OntologyReference`, `EntityReference`, `EntityId`, `EntityType`,
`KnownEntityType`, `EntityData`, and `ConceptType` — the types shared by
`mif-schema` (JSON Schema validation) and `mif-ontology` (ontology
resolution). Field definitions are taken directly from the canonical MIF
JSON Schema at <https://mif-spec.dev/schema/>.

`EntityId` is the entity identifier object nested inside an
`EntityReference` (its `@id` field). `EntityType` classifies an entity as
either a closed, well-known type or a custom ontology-defined one: it is a
`Known(KnownEntityType) | Custom(String)` enum (via `#[serde(untagged)]`)
rather than a plain closed enum, so that a custom type from an ontology
(e.g. `grazing-plan`, `soil-profile`) round-trips verbatim instead of being
discarded. `KnownEntityType` is that closed set of well-known values
(`Person`, `Organization`, `Technology`, `Concept`, `File`).

## License

MIT
