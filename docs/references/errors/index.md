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
- [level-floor-violation/v1](/mif-rs/references/errors/level-floor-violation/v1/)
- [unsupported-level/v1](/mif-rs/references/errors/unsupported-level/v1/)

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
- [unknown-frontmatter-shape/v1](/mif-rs/references/errors/unknown-frontmatter-shape/v1/)

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

`invalid-json`, `document-not-found`, and the per-binary
`mif-cli-json-serialize-failure`/`mif-mcp-json-serialize-failure` pair are
the only problem types unique to `mif-cli`/`mif-mcp` — they also
independently define their own `io` problem type (shared with
`mif-ontology`; see [io/v1](/mif-rs/references/errors/io/v1/) above) rather
than delegating an I/O failure through the library layer. Every other
failure they report delegates verbatim to the wrapped library crate's own
problem type.

- [invalid-json/v1](/mif-rs/references/errors/invalid-json/v1/)
- [document-not-found/v1](/mif-rs/references/errors/document-not-found/v1/)
- [mif-cli-json-serialize-failure/v1](/mif-rs/references/errors/mif-cli-json-serialize-failure/v1/)
- [mif-mcp-json-serialize-failure/v1](/mif-rs/references/errors/mif-mcp-json-serialize-failure/v1/)

## Research-harness engine (`mif-rh`)

`mif-rh` is the compiled research-harness ontology engine — deterministic
`resolve()`/`review()`, the hypothesis layer (`suggest_type()`, the
`FindingIndex`, the suggestion queue), and confidence calibration. Three of
its `MifRhError` variants (`Ontology`, `Frontmatter`, `Embed`) wrap and
delegate verbatim to `mif-ontology`/`mif-frontmatter`/`mif-embed`'s own
problem types rather than defining their own — those wrapped variants are
not listed again below.

- [finding-io/v1](/mif-rs/references/errors/finding-io/v1/)
- [finding-invalid-json/v1](/mif-rs/references/errors/finding-invalid-json/v1/)
- [mif-rh-io/v1](/mif-rs/references/errors/mif-rh-io/v1/)
- [mif-rh-invalid-json/v1](/mif-rs/references/errors/mif-rh-invalid-json/v1/)
- [json-serialize-failure/v1](/mif-rs/references/errors/json-serialize-failure/v1/)
- [ontology-pack-invalid-yaml/v1](/mif-rs/references/errors/ontology-pack-invalid-yaml/v1/)
- [frontmatter-yaml-serialize-failure/v1](/mif-rs/references/errors/frontmatter-yaml-serialize-failure/v1/)
- [catalog-missing/v1](/mif-rs/references/errors/catalog-missing/v1/)
- [config-missing/v1](/mif-rs/references/errors/config-missing/v1/)
- [direct-binding-invalid/v1](/mif-rs/references/errors/direct-binding-invalid/v1/)
- [schema-compilation-failed/v1](/mif-rs/references/errors/schema-compilation-failed/v1/)
- [ref-schema-missing-id/v1](/mif-rs/references/errors/ref-schema-missing-id/v1/)
- [schema-validation-failed/v1](/mif-rs/references/errors/schema-validation-failed/v1/)
- [invalid-toggle-value/v1](/mif-rs/references/errors/invalid-toggle-value/v1/)
- [empty-source-content/v1](/mif-rs/references/errors/empty-source-content/v1/)
- [pack-not-declared/v1](/mif-rs/references/errors/pack-not-declared/v1/)
- [no-findings-found/v1](/mif-rs/references/errors/no-findings-found/v1/)
- [no-surviving-findings/v1](/mif-rs/references/errors/no-surviving-findings/v1/)
- [artifact-not-publishable/v1](/mif-rs/references/errors/artifact-not-publishable/v1/)
- [missing-provenance/v1](/mif-rs/references/errors/missing-provenance/v1/)
- [reconcile-environment-broken/v1](/mif-rs/references/errors/reconcile-environment-broken/v1/)
- [topic-not-registered/v1](/mif-rs/references/errors/topic-not-registered/v1/)
- [invalid-concordance/v1](/mif-rs/references/errors/invalid-concordance/v1/)
- [ontology-map-unusable/v1](/mif-rs/references/errors/ontology-map-unusable/v1/)
- [relationship-target-finding-unparseable/v1](/mif-rs/references/errors/relationship-target-finding-unparseable/v1/)
- [subtype-of-cycle/v1](/mif-rs/references/errors/subtype-of-cycle/v1/)
- [entity-type-schema-invalid/v1](/mif-rs/references/errors/entity-type-schema-invalid/v1/)
- [index-failure/v1](/mif-rs/references/errors/index-failure/v1/)
- [lock-io/v1](/mif-rs/references/errors/lock-io/v1/)
- [lock-held/v1](/mif-rs/references/errors/lock-held/v1/)
- [queue-topic-mismatch/v1](/mif-rs/references/errors/queue-topic-mismatch/v1/)
- [registry-fetch-failed/v1](/mif-rs/references/errors/registry-fetch-failed/v1/)
- [registry-index-invalid/v1](/mif-rs/references/errors/registry-index-invalid/v1/)
- [ontology-not-in-registry/v1](/mif-rs/references/errors/ontology-not-in-registry/v1/)
- [lock-source-mismatch/v1](/mif-rs/references/errors/lock-source-mismatch/v1/)
- [ontology-pack-not-utf8/v1](/mif-rs/references/errors/ontology-pack-not-utf8/v1/)
- [index-pin-mismatch/v1](/mif-rs/references/errors/index-pin-mismatch/v1/)
- [ontology-checksum-mismatch/v1](/mif-rs/references/errors/ontology-checksum-mismatch/v1/)
- [unsafe-index-path/v1](/mif-rs/references/errors/unsafe-index-path/v1/)
- [malformed-ontology-id/v1](/mif-rs/references/errors/malformed-ontology-id/v1/)
- [config-malformed/v1](/mif-rs/references/errors/config-malformed/v1/)
- [no-entity-types-found/v1](/mif-rs/references/errors/no-entity-types-found/v1/)
- [no-clusters-found/v1](/mif-rs/references/errors/no-clusters-found/v1/)
- [version-not-semver/v1](/mif-rs/references/errors/version-not-semver/v1/)
- [version-missing/v1](/mif-rs/references/errors/version-missing/v1/)
- [version-unchanged/v1](/mif-rs/references/errors/version-unchanged/v1/)
- [pack-not-found/v1](/mif-rs/references/errors/pack-not-found/v1/)
- [pack-ambiguous/v1](/mif-rs/references/errors/pack-ambiguous/v1/)
- [pack-file-missing/v1](/mif-rs/references/errors/pack-file-missing/v1/)
- [pack-version-invalid/v1](/mif-rs/references/errors/pack-version-invalid/v1/)
- [pack-ahead-of-release/v1](/mif-rs/references/errors/pack-ahead-of-release/v1/)
- [changelog-anchor-missing/v1](/mif-rs/references/errors/changelog-anchor-missing/v1/)
- [verification-failed/v1](/mif-rs/references/errors/verification-failed/v1/)
