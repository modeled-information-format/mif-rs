---
id: reference-spell-check-workflow
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: reference/workflows
title: spell-check.yml — GitHub Actions workflow reference
tags:
  - reference
  - ci
  - workflow
  - spell-check
  - typos
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
    '@id': https://github.com/modeled-information-format/mif-rs/blob/main/.github/workflows/spell-check.yml
    '@type': prov:Entity
citations:
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: typos
    url: https://github.com/crate-ci/typos
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
  name: spell-check.yml
  entity_type: reference-document
---

# spell-check.yml

The `.github/workflows/spell-check.yml` workflow ("Spell Check") runs
`crate-ci/typos` over the repository directly (not via a reusable workflow)
and warns on typos without failing CI.

## Synopsis

```yaml
on:
  pull_request:
  push:
    branches: [main, master]
  workflow_dispatch:
```

## Triggers

| Event | Condition |
| --- | --- |
| `pull_request` | Any branch |
| `push` | Branch `main` or `master` |
| `workflow_dispatch` | Manual |

## Permissions

| Scope | Level |
| --- | --- |
| `contents` | `read` |

## Job

| Job ID | Name | Runs on |
| --- | --- | --- |
| `typos` | Check Spelling | `ubuntu-latest` |

## Steps

| Step | Action | Pin | `with:` |
| --- | --- | --- | --- |
| Checkout repository | `actions/checkout` | `9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0` (v7.0.0) | — |
| Check spelling with typos | `crate-ci/typos` | `37bb98842b0d8c4ffebdb75301a13db0267cef89` (master, latest) | `files: .`, `config: .typos.toml`; step has `continue-on-error: true` |

## Failure behavior

| Condition | Effect |
| --- | --- |
| Typo found | Step reports failure internally but `continue-on-error: true` means the job — and the workflow — still succeeds |

## Configuration file

`.typos.toml` (repository root):

| Section | Setting | Value |
| --- | --- | --- |
| `[default].extend-ignore-re` | Ignored patterns | `[0-9a-f]{40}` (git SHAs), UUID pattern |
| `[files].extend-exclude` | Excluded paths | `target/`, `*.lock`, `*.svg`, `.git/` |
| `[default.extend-words]` | Project dictionary | Empty (no entries defined) |

## Warning output

```text
warning: `recieve` should be `receive`
  --> crates/mif-core/src/entity.rs:10
```

Results are visible in the Actions tab (job summary), not the Security tab —
`typos` does not emit SARIF in this workflow.

## Examples

Reproduce locally:

```bash
cargo install typos-cli
typos
typos --write-changes
```
