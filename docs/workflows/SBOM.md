---
id: reference-sbom-workflow
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: reference/workflows
title: release.yml sbom job — GitHub Actions workflow reference
tags:
  - reference
  - ci
  - workflow
  - sbom
  - release
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
    '@id': https://github.com/modeled-information-format/mif-rs/blob/main/.github/workflows/release.yml
    '@type': prov:Entity
citations:
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: anchore/sbom-action
    url: https://github.com/anchore/sbom-action
  - '@type': Citation
    citationType: specification
    citationRole: source
    title: CycloneDX Specification
    url: https://cyclonedx.org/specification/overview/
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
  name: release.yml sbom job
  entity_type: reference-document
---

# release.yml — sbom job

The `sbom` job in `.github/workflows/release.yml` generates a CycloneDX JSON
Software Bill of Materials over the release's built binaries and attests it,
binding the SBOM to every published binary artifact.

## Synopsis

```yaml
on:
  push:
    tags: ["v*.*.*"]
  workflow_dispatch:
```

## Triggers

| Event | Condition |
| --- | --- |
| `push` (tag) | Tag matches `v*.*.*` |
| `workflow_dispatch` | Manual dry-run from any branch (produces no persistent attestation) |

## Job

| Job ID | Name | Runs on | `needs` |
| --- | --- | --- | --- |
| `sbom` | SBOM (generate + attest) | `ubuntu-latest` | `[meta, build]` |

## Permissions

| Scope | Level |
| --- | --- |
| `contents` | `read` |
| `id-token` | `write` |
| `attestations` | `write` |

## Steps

| Step | Action | Pin | Effect |
| --- | --- | --- | --- |
| Checkout repository | `actions/checkout` | `9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0` (v7.0.0) | Fetch source |
| Download all binaries | `actions/download-artifact` | `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` (v8.0.1) | Pattern `*-${version}-*-*` into `dist/` — a wildcard bin prefix, since it must match every `[[bin]]` target's platform binaries (`mif-cli`, `mif-mcp`), not one fixed name |
| Generate CycloneDX SBOM | `anchore/sbom-action` | `e22c389904149dbc22b58101806040fa8d37a610` (v0.24.0) | `path: .`, `format: cyclonedx-json`, output `mif-rs-${version}-sbom.cdx.json` — one combined SBOM covering the whole workspace, not per-binary |
| Attest SBOM | `actions/attest-sbom` | `c604332985a26aa8cf1bdc465b92731239ec6b9e` (v4.1.0) | Tag-only (`if: startsWith(github.ref, 'refs/tags/')`); `subject-path: dist/*`, binds every downloaded binary (both `mif-cli` and `mif-mcp`, all 5 platforms) to the one SBOM |
| Upload SBOM artifact | `actions/upload-artifact` | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` (v7.0.1) | Name `mif-rs-${version}-sbom` |

## Binary name / version resolution

Resolved once in the upstream `meta` job (`needs: [meta, build]`), from
`cargo metadata --no-deps --locked --format-version 1`:

| Output | Source | Logic |
| --- | --- | --- |
| `bins` | `.packages[].targets[]` | Every `[[bin]]` target across every workspace member (`jq -c '[.packages[].targets[] | select(.kind | index("bin")) | .name] | unique'`) — never `.packages[0]`, which would only see one crate |
| `bin-count` | `.bins | length` | Used by the downstream `verify` job to compute the expected artifact count dynamically |
| `version` | `GITHUB_REF` or `.packages[0].version` | `refs/tags/v*` → strip the `v` prefix; otherwise (`workflow_dispatch`) `<Cargo.toml version>-dev`. Reading `.packages[0].version` here is safe specifically because every workspace member shares `version.workspace = true` — unlike `bins`, which genuinely differs per crate and must never be read positionally |

**Current repository state**: `cargo metadata` resolves `bins` to
`["mif-cli", "mif-mcp"]` in this workspace — both are real `[[bin]]` targets
from day one. `meta` fails only if the *entire* workspace has zero `[[bin]]`
targets (`::error::no [[bin]] target found in the workspace`), which is not
the case here. The `build` job matrixes over `bin x platform` (2 bins x 5
platforms = 10 build legs), so `sbom` downloads and attests binaries for
both `mif-cli` and `mif-mcp`.

## SBOM contents (shape)

```json
{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "metadata": {
    "component": { "type": "application", "name": "mif-cli", "version": "0.1.0" }
  },
  "components": [
    {
      "type": "library",
      "name": "serde",
      "version": "1.0.228",
      "licenses": [{ "license": { "id": "MIT" } }],
      "purl": "pkg:cargo/serde@1.0.228"
    }
  ]
}
```

## Artifacts

| Name | Path | Notes |
| --- | --- | --- |
| `mif-rs-${version}-sbom` | `mif-rs-${version}-sbom.cdx.json` | Also attached to the GitHub Release by the downstream `release` job |

## Attestation

| Field | Value |
| --- | --- |
| Predicate type | `https://cyclonedx.org/bom` |
| Subject | Each file under `dist/*` (every platform binary, both `mif-cli` and `mif-mcp`) |
| Verify | `gh attestation verify <binary> --repo modeled-information-format/mif-rs --predicate-type https://cyclonedx.org/bom` |

## Examples

Generate an equivalent CycloneDX SBOM locally with Syft (the engine behind
`anchore/sbom-action`):

```bash
curl -sSfL https://raw.githubusercontent.com/anchore/syft/main/install.sh | sh -s -- -b /usr/local/bin
syft dir:. -o cyclonedx-json > sbom.cdx.json
```
