# mif-mcp

MCP server for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Exposes two tools over stdio, mirroring `mif-cli`:

- `validate_mif_document` — validate a MIF document against the canonical schema.
- `resolve_ontology_reference` — resolve an ontology's three-tier `extends` chain.

## License

MIT
