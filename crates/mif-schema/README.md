# mif-schema

JSON Schema validation for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Validates MIF documents and citation objects against the canonical MIF JSON
Schema (draft 2020-12). Schemas are vendored at compile time and resolved
entirely offline.

Three validation functions cover the schema surface: `validate_document`
validates a MIF document (a JSON-LD-projected memory) against
`mif.schema.json`, `validate_citation` validates a standalone MIF citation
object against `citation.schema.json`, and `validate_ontology_definition`
validates an ontology definition object against `ontology.schema.json`.

`MifSchemaError` implements `mif_problem::ToProblem`, mapping both its
variants — `Invalid` (the instance failed schema validation) and
`SchemaCompilation` (the vendored schema itself failed to compile,
indicating a bug in this crate rather than the instance) — to distinct RFC
9457 `application/problem+json` representations.

## License

MIT
