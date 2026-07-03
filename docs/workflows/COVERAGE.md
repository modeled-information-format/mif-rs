---
id: reference-coverage-workflow
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: reference/workflows
title: ci-coverage.yml — GitHub Actions workflow reference
tags:
  - reference
  - ci
  - workflow
  - coverage
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
provenance:
  '@type': Provenance
  sourceType: system_generated
  trustLevel: verified
  wasDerivedFrom:
    '@id': https://github.com/modeled-information-format/mif-rs/blob/main/.github/workflows/ci-coverage.yml
    '@type': prov:Entity
citations:
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: cargo-llvm-cov
    url: https://github.com/taiki-e/cargo-llvm-cov
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: Codecov
    url: https://docs.codecov.com/
  - '@type': Citation
    citationType: specification
    citationRole: methodology
    title: Diátaxis — Reference
    url: https://diataxis.fr/reference/
    accessed: '2026-07-02'
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: ci-coverage.yml
  entity_type: reference-document
---

# ci-coverage.yml

The `.github/workflows/ci-coverage.yml` workflow ("Code Coverage") measures
workspace-wide test coverage with `cargo-llvm-cov`, enforces a 90% threshold,
and optionally uploads to Codecov.

## Synopsis

```yaml
on:
  workflow_call:
    secrets:
      CODECOV_TOKEN:
        required: false
  workflow_dispatch:
```

This is a `workflow_call` target with no direct `push`/`pull_request`
triggers of its own; it is invoked by `.github/workflows/pipeline.yml`'s
`coverage` job.

## Triggers

| Event | Condition |
| --- | --- |
| `workflow_call` | Optional `CODECOV_TOKEN` secret passthrough |
| `workflow_dispatch` | Manual, direct invocation |

Via `pipeline.yml`, the `coverage` job runs unless the run is a
`workflow_dispatch` with `inputs.stage` set to something other than `all` or
`ci` — in practice, on every `push`, `pull_request`, and tag push that
`pipeline.yml` itself triggers on.

## Permissions

| Scope | Level |
| --- | --- |
| `contents` | `read` |

## Job

| Job ID | Name | Runs on | Timeout |
| --- | --- | --- | --- |
| `coverage` | Generate Coverage Report | `ubuntu-latest` | 30 minutes |

## Steps

| Step | Action / command | Pin |
| --- | --- | --- |
| Checkout repository | `actions/checkout` | `9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0` (v7.0.0) |
| Install Rust toolchain | `dtolnay/rust-toolchain`, `toolchain: stable`, `components: llvm-tools-preview` | `fa04a1451ff1842e2626ccb99004d0195b455a88` |
| Install cargo-llvm-cov | `taiki-e/install-action`, `tool: cargo-llvm-cov@0.6.14` | `16b05812d776ae1dfaabc8277e421fb6d2506419` |
| Cache cargo registry | `actions/cache` | `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` (v5.0.3) |
| Generate coverage | `cargo llvm-cov --all-features --workspace --lcov/--html/--json` (three invocations) | — |
| Parse coverage percentage | `jq '.data[0].totals.lines.percent * 100 \| round / 100' coverage.json` | — |
| Generate coverage report | Writes `coverage-report.md` (overall %, `cargo llvm-cov --summary-only` embedded) | — |
| Upload coverage to Codecov | `codecov/codecov-action`, `files: lcov.info`, `fail_ci_if_error: false` | `fb8b3582c8e4def4969c97caa2f19720cb33a72f` (v5.5.2) |
| Upload coverage artifacts | `actions/upload-artifact` | `47309c993abb98030a35d55ef7ff34b7fa1074b5` (v4.6.2) |
| Check coverage threshold | Fails if `COVERAGE < 90` (via `bc`) | — |

## Inputs

| Name | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `CODECOV_TOKEN` (secret) | string | no | none | Codecov upload token. Upload step still runs without it (`fail_ci_if_error: false`); Codecov ingestion may reject an unauthenticated upload depending on repo visibility. |

## Threshold

| Threshold | Value | Enforcement |
| --- | --- | --- |
| Minimum line coverage | 90% | Job exits non-zero when `COVERAGE < 90` |

## Coverage types measured

| Type | Description |
| --- | --- |
| Line coverage | Percent of lines executed |
| Branch coverage | Percent of conditional branches taken |
| Function coverage | Percent of functions called |

## Artifacts

| Name | Contents | Retention |
| --- | --- | --- |
| `coverage-report` | `coverage-report.md`, `coverage-html/`, `lcov.info`, `coverage.json` | 30 days |

## Report format

```text
Filename                       Regions  Missed Regions  Coverage
---------------------------------------------------------------
crates/mif-core/src/entity.rs       45               3     93.33%
crates/mif-schema/src/lib.rs        78              12     84.62%
crates/mif-ontology/src/lib.rs      23               0    100.00%
---------------------------------------------------------------
TOTAL                              146              15     89.73%
```

## Examples

Reproduce locally:

```bash
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov
cargo llvm-cov --all-features --workspace --summary-only
```
