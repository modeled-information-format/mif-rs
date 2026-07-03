---
id: reference-container-scan-workflow
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: reference/workflows
title: Container vulnerability scanning (Trivy) — GitHub Actions workflow reference
tags:
  - reference
  - ci
  - workflow
  - trivy
  - container
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
    '@id': https://github.com/modeled-information-format/mif-rs/blob/main/.github/workflows/quality-gates.yml
    '@type': prov:Entity
citations:
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: Trivy
    url: https://aquasecurity.github.io/trivy/
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
  name: Container vulnerability scanning (Trivy)
  entity_type: reference-document
---

# Container vulnerability scanning (Trivy)

Trivy scanning in this repo has no dedicated workflow file. It runs through
the central reusable `reusable-trivy.yml` (from
`modeled-information-format/.github`), called from two jobs in two different
caller workflows, plus one attestation job.

## Callers

| Caller workflow | Job ID | Job name | Scan target | Gated by |
| --- | --- | --- | --- | --- |
| `.github/workflows/quality-gates.yml` | `trivy` | (unnamed) | Filesystem (source tree: Dockerfile, manifests, licenses) | Always runs (see Triggers) |
| `.github/workflows/pipeline.yml` | `gate-image` | Gate — Trivy (image) | One representative built container image, by digest | `needs: [docker]`; `docker` itself gated on `needs.gate.outputs.has-bin-target == 'true'` |
| `.github/workflows/pipeline.yml` | `attest-container-scan` | Attest — Container scan | Signs the `gate-image` verdict | `needs: [docker, gate-image]` |

## Triggers

The `quality-gates.yml` filesystem scan inherits that workflow's top-level triggers:

| Event | Condition |
| --- | --- |
| `push` | Branch `main` |
| `pull_request` | Target branch `main` |
| `schedule` | `0 6 * * 1` (Monday 06:00 UTC) |
| `workflow_dispatch` | Manual |

The `gate-image`/`attest-container-scan` jobs in `pipeline.yml` are event-driven
(not scheduled): they run when `github.event_name != 'pull_request'` and,
for `workflow_dispatch`, only when `inputs.stage` is `all` or `docker`. They
additionally require `needs.gate.outputs.has-bin-target == 'true'` — true from
day one in this workspace, since `mif-cli` and `mif-mcp` are real `[[bin]]`
targets (see "Current repository state" below for why this differs from the
`crates-publishable` gate that governs crates.io/Homebrew).

## Reusable workflow invocations

| Job | Reusable | Pin | `with:` |
| --- | --- | --- | --- |
| `trivy` (quality-gates.yml) | `reusable-trivy.yml` | `e50b004cbdcf2b3258d223b1f6a4d98ff7938abf` | `scan-iac: true` (no `image-ref`; filesystem mode) |
| `gate-image` (pipeline.yml) | `reusable-trivy.yml` | `e50b004cbdcf2b3258d223b1f6a4d98ff7938abf` | `image-ref: ghcr.io/${{ github.repository }}/${{ fromJson(needs.docker.outputs.bins)[0] }}@<that bin's digest>`, `scan-iac: false` |
| `attest-container-scan` (pipeline.yml) | `reusable-attest-scan.yml` | `e50b004cbdcf2b3258d223b1f6a4d98ff7938abf` | `subject-name: ghcr.io/${{ github.repository }}/<same first bin>`, `subject-digest: <that bin's digest>`, `predicate-type: https://modeled-information-format.github.io/attestations/container-scan/v1` |

## Permissions

| Job | `contents` | `security-events` | `actions` | `packages` | `id-token` | `attestations` |
| --- | --- | --- | --- | --- | --- | --- |
| `trivy` | `read` | `write` | `read` | `read` | — | — |
| `gate-image` | `read` | `write` | `read` | `read` | — | — |
| `attest-container-scan` | `read` | — | — | — | `write` | `write` |

## What it scans for

| Category | Coverage |
| --- | --- |
| OS package vulnerabilities | CVEs in image base-layer packages |
| Application dependencies | Vulnerabilities in bundled application dependencies |
| Misconfigurations | Dockerfile/IaC misconfiguration rules (filesystem scan, `scan-iac: true`) |
| Secrets | Secrets embedded in image layers |

Severity levels: `CRITICAL`, `HIGH`, `MEDIUM`, `LOW`, `UNKNOWN`.

## Current repository state

`docker`, `gate-image`, and `attest-container-scan` are gated on
`needs.gate.outputs.has-bin-target == 'true'`, resolved dynamically from
`cargo metadata` (any workspace member with a `[[bin]]` target). `mif-cli`
and `mif-mcp` are real binaries from day one in this workspace, so this gate
is `true` immediately — the container chain is **active**, not dormant. This
is a deliberate split from the separate `crates-publishable` gate (any
member with `publish != false`), which governs crates.io/Homebrew instead:
the libraries (`mif-core`, `mif-schema`, `mif-ontology`) can go
`publish = true` long before that has any bearing on whether there's a
binary to containerize, and vice versa.

`gate-image`/`attest-container-scan` deliberately scan and attest only the
**first** resolved bin (`fromJson(needs.docker.outputs.bins)[0]`), not both
`mif-cli` and `mif-mcp` — `reusable-trivy.yml` uploads its SARIF under a
fixed artifact name (`container-scan-sarif`), so a second matrix cell
calling it in the same run would collide on that name. Both images share the
same base image (`chainguard/glibc-dynamic`) and the same dependency tree (the
Dockerfile builds from one workspace checkout), so scanning one is
representative of the other for base-image/dependency CVEs. The filesystem
scan (`trivy` job in `quality-gates.yml`) is unaffected by any of this and
runs unconditionally on its own triggers.

## Outputs (SARIF)

Filesystem findings upload to **Security tab → Code scanning alerts**. Image
scan findings become the `gate-image.outputs.image-sarif-artifact` /
`image-sarif-filename` pair, consumed by `attest-container-scan` and signed as
a `container-scan/v1` attestation bound to the image digest — verifiable with
`gh attestation verify`.

```json
{
  "results": [
    {
      "ruleId": "CVE-2021-12345",
      "level": "error",
      "message": { "text": "openssl: buffer overflow vulnerability" },
      "locations": [
        { "physicalLocation": { "artifactLocation": { "uri": "Dockerfile" } } }
      ]
    }
  ]
}
```

## Examples

Reproduce the filesystem scan locally:

```bash
trivy fs --scanners vuln,misconfig,secret .
```

Reproduce the image scan locally, against a locally built image (the
Dockerfile requires a `BIN` build-arg selecting which workspace binary to
build in):

```bash
docker build --build-arg BIN=mif-cli -t mif-cli:local .
trivy image mif-cli:local
```
