---
id: reference-code-quality-workflow
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: reference/workflows
title: code-quality.yml — GitHub Actions workflow reference
tags:
  - reference
  - ci
  - workflow
  - code-quality
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
    '@id': https://github.com/modeled-information-format/mif-rs/blob/main/.github/workflows/code-quality.yml
    '@type': prov:Entity
citations:
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: cargo-geiger
    url: https://github.com/rust-secure-code/cargo-geiger
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: cargo-bloat
    url: https://github.com/RazrFalcon/cargo-bloat
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
  name: code-quality.yml
  entity_type: reference-document
---

# code-quality.yml

The `.github/workflows/code-quality.yml` workflow ("Code Quality Metrics")
collects unsafe-code, binary-size, and documentation-coverage metrics for the
workspace and publishes them as a single markdown report artifact.

## Synopsis

```yaml
on:
  pull_request:
    branches: [main, master]
  workflow_dispatch:
```

## Triggers

| Event | Condition |
| --- | --- |
| `pull_request` | Target branch `main` or `master` |
| `workflow_dispatch` | Manual, any branch |

## Permissions

| Scope | Level |
| --- | --- |
| `contents` | `read` |

## Job

| Job ID | Name | Runs on |
| --- | --- | --- |
| `metrics` | Collect Code Quality Metrics | `ubuntu-latest` |

## Steps

| Step | Action | Pin | Command / effect |
| --- | --- | --- | --- |
| Checkout repository | `actions/checkout` | `9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0` (v7.0.0) | Fetch source |
| Install Rust toolchain | `dtolnay/rust-toolchain` | `fa04a1451ff1842e2626ccb99004d0195b455a88` | Install `stable` toolchain |
| Install analysis tools | `taiki-e/install-action` | `16b05812d776ae1dfaabc8277e421fb6d2506419` (v2.67.18) | Install `cargo-geiger`, `cargo-bloat` |
| Cache cargo registry | `actions/cache` | `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` (v5.0.3) | Cache `~/.cargo/registry`, `~/.cargo/git`, `target`, keyed on `hashFiles('**/Cargo.lock')` |
| Scan for unsafe code | (inline `run`) | — | `cargo geiger --all-features`, appended under `## Unsafe Code Analysis` |
| Analyze binary size | (inline `run`) | — | `cargo build --release` then `cargo bloat --release --crates`, appended under `## Binary Size Analysis` |
| Check documentation coverage | (inline `run`) | — | `cargo doc --no-deps --all-features 2>&1 \| grep -E "Documenting\|warning"`, appended under `## Documentation Coverage` |
| Upload metrics report | `actions/upload-artifact` | `47309c993abb98030a35d55ef7ff34b7fa1074b5` (v4.6.2) | Upload `metrics-report.md` |

## Metrics collected

| Metric | Tool | Detects |
| --- | --- | --- |
| Unsafe code analysis | `cargo-geiger` | Unsafe functions, expressions, trait impls, and unsafe usage transitively through dependencies |
| Binary size analysis | `cargo-bloat` | Binary size contribution by crate and by function |
| Documentation coverage | `rustdoc` (via `cargo doc`) | Missing doc comments, broken intra-doc links, doc-test failures |

## Artifacts

| Name | Path | Retention |
| --- | --- | --- |
| `code-quality-metrics` | `metrics-report.md` | 30 days |

Access: **Actions → Code Quality Metrics → Artifacts → code-quality-metrics**.

## Report format

```markdown
## Unsafe Code Analysis
Functions  Expressions  Impls  Traits  Methods  Dependency
0/10       0/100        0/5    0/2     0/20     mif_core

## Binary Size Analysis
File   .text   Size    Crate
 71.0%  59.0%   1.2MiB  std
  8.5%   7.1%   147KiB  mif_core

## Documentation Coverage
Documenting mif_core v0.1.0
warning: missing documentation for public function
  --> crates/mif-core/src/entity.rs:10
```

## Examples

Run the same three checks locally:

```bash
cargo geiger --all-features
cargo build --release && cargo bloat --release --crates
cargo doc --no-deps --all-features
```
