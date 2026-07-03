---
title: "Error reference"
description: "The RFC 9457 Problem Details catalog that every mif-rs crate emits."
---

Every library crate in this workspace (`mif-schema`, `mif-ontology`,
`mif-frontmatter`, `mif-embed`, `mif-store`) and both binaries (`mif-cli`,
`mif-mcp`) map their errors to an [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457)
`application/problem+json` envelope via the shared `mif_problem` crate. Each
problem type gets a stable, versioned `type` URI of the form:

```
https://modeled-information-format.github.io/mif-rs/references/errors/{slug}/{version}
```

That URI is dereferenceable: it resolves to the reference page documenting
that exact problem type, one page per `{slug}/{version}` pair below.

## Schema validation (`mif-schema`)

- [schema-compilation/v1](/mif-rs/references/errors/schema-compilation/v1/)
- [invalid-document/v1](/mif-rs/references/errors/invalid-document/v1/)

## Ontology resolution (`mif-ontology`)

- [io/v1](/mif-rs/references/errors/io/v1/)
- [invalid-yaml/v1](/mif-rs/references/errors/invalid-yaml/v1/)
- [invalid-ontology-definition/v1](/mif-rs/references/errors/invalid-ontology-definition/v1/)
- [ontology-metadata-mismatch/v1](/mif-rs/references/errors/ontology-metadata-mismatch/v1/)
- [ontology-not-found/v1](/mif-rs/references/errors/ontology-not-found/v1/)
- [ontology-extends-cycle/v1](/mif-rs/references/errors/ontology-extends-cycle/v1/)

## Frontmatter projection (`mif-frontmatter`)

- [missing-frontmatter/v1](/mif-rs/references/errors/missing-frontmatter/v1/)
- [invalid-frontmatter-yaml/v1](/mif-rs/references/errors/invalid-frontmatter-yaml/v1/)
- [frontmatter-not-a-mapping/v1](/mif-rs/references/errors/frontmatter-not-a-mapping/v1/)
- [yaml-serialization-failure/v1](/mif-rs/references/errors/yaml-serialization-failure/v1/)
- [field-json-conversion-failure/v1](/mif-rs/references/errors/field-json-conversion-failure/v1/)
- [field-yaml-conversion-failure/v1](/mif-rs/references/errors/field-yaml-conversion-failure/v1/)
- [field-not-a-string/v1](/mif-rs/references/errors/field-not-a-string/v1/)
- [jsonld-not-an-object/v1](/mif-rs/references/errors/jsonld-not-an-object/v1/)
- [json-roundtrip-failure/v1](/mif-rs/references/errors/json-roundtrip-failure/v1/)
- [roundtrip-drift/v1](/mif-rs/references/errors/roundtrip-drift/v1/)

## Embedding (`mif-embed`)

- [no-cache-dir/v1](/mif-rs/references/errors/no-cache-dir/v1/)
- [hub-client-init-failure/v1](/mif-rs/references/errors/hub-client-init-failure/v1/)
- [model-fetch-failure/v1](/mif-rs/references/errors/model-fetch-failure/v1/)
- [read-cached-model-file-failure/v1](/mif-rs/references/errors/read-cached-model-file-failure/v1/)
- [invalid-model-config/v1](/mif-rs/references/errors/invalid-model-config/v1/)
- [load-tokenizer-failure/v1](/mif-rs/references/errors/load-tokenizer-failure/v1/)
- [load-model-weights-failure/v1](/mif-rs/references/errors/load-model-weights-failure/v1/)
- [tokenize-failure/v1](/mif-rs/references/errors/tokenize-failure/v1/)
- [inference-failure/v1](/mif-rs/references/errors/inference-failure/v1/)

## Vector store (`mif-store`)

- [missing-parent-dir/v1](/mif-rs/references/errors/missing-parent-dir/v1/)
- [open-database-failure/v1](/mif-rs/references/errors/open-database-failure/v1/)
- [sqlite-query-failure/v1](/mif-rs/references/errors/sqlite-query-failure/v1/)
- [dimension-too-large/v1](/mif-rs/references/errors/dimension-too-large/v1/)
- [corrupt-dimension/v1](/mif-rs/references/errors/corrupt-dimension/v1/)
- [vector-blob-mismatch/v1](/mif-rs/references/errors/vector-blob-mismatch/v1/)

## CLI and MCP server (`mif-cli`, `mif-mcp`)

`invalid-json` and `document-not-found` are the only problem types unique to
`mif-cli`/`mif-mcp` — they also independently define their own `io` problem
type (shared with `mif-ontology`; see [io/v1](/mif-rs/references/errors/io/v1/)
above) rather than delegating an I/O failure through the library layer. Every
other failure they report delegates verbatim to the wrapped library crate's
own problem type.

- [invalid-json/v1](/mif-rs/references/errors/invalid-json/v1/)
- [document-not-found/v1](/mif-rs/references/errors/document-not-found/v1/)
