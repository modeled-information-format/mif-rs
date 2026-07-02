---
id: reference-secrets-scan-workflow
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: reference/workflows
title: secrets-scan.yml — GitHub Actions workflow reference
tags:
  - reference
  - ci
  - workflow
  - secrets
  - gitleaks
  - trufflehog
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
    '@id': https://github.com/modeled-information-format/mif-rs/blob/main/.github/workflows/secrets-scan.yml
    '@type': prov:Entity
citations:
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: Gitleaks
    url: https://github.com/gitleaks/gitleaks
  - '@type': Citation
    citationType: tool
    citationRole: source
    title: TruffleHog
    url: https://github.com/trufflesecurity/trufflehog
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
  name: secrets-scan.yml
  entity_type: reference-document
---

# secrets-scan.yml

The `.github/workflows/secrets-scan.yml` workflow ("Secrets Scan") is a thin
caller of the central `reusable-secrets.yml` (from
`modeled-information-format/.github`), which runs Gitleaks and TruffleHog
over the repository. It replaces the licensed `gitleaks-action` (which
requires a paid `GITLEAKS_LICENSE` for org repos) with checksum-verified
release binaries of both tools.

## Synopsis

```yaml
on:
  push:
  pull_request:
  workflow_dispatch:

jobs:
  secrets:
    uses: modeled-information-format/.github/.github/workflows/reusable-secrets.yml@<sha>
```

## Triggers

| Event | Condition |
| --- | --- |
| `push` | Any branch |
| `pull_request` | Any branch |
| `workflow_dispatch` | Manual |

## Job

| Job ID | `uses` | Pin |
| --- | --- | --- |
| `secrets` | `modeled-information-format/.github/.github/workflows/reusable-secrets.yml` | `e50b004cbdcf2b3258d223b1f6a4d98ff7938abf` |

## Permissions (caller)

| Scope | Level |
| --- | --- |
| `contents` | `read` |
| `security-events` | `write` |
| `actions` | `read` |

## Reusable workflow inputs

| Name | Type | Default | Description |
| --- | --- | --- | --- |
| `directory` | string | `.` | Directory to scan (not overridden by the caller, so `.`) |
| `gitleaks-version` | string | `8.30.1` | Pinned Gitleaks release (not overridden) |
| `trufflehog-version` | string | `3.95.6` | Pinned TruffleHog release (not overridden) |
| `fail-on-verified` | boolean | `true` | Fail the job if TruffleHog confirms a live secret (not overridden) |

## Reusable workflow outputs

| Name | Value | Description |
| --- | --- | --- |
| `sarif-artifact` | `secrets-sarif` | Artifact name holding the Gitleaks SARIF |
| `sarif-filename` | `gitleaks.sarif` | SARIF filename within that artifact |

## Steps (inside the reusable job)

| Step | Effect |
| --- | --- |
| Checkout | Fetch source |
| Install Gitleaks + TruffleHog | Downloads both tools' Linux x64 release tarballs directly from GitHub Releases and verifies each against its published `sha256sum` checksums file before extracting — no third-party Action |
| Gitleaks → SARIF (soft-fail) | `gitleaks dir "${SCAN_DIR}" --report-format sarif --report-path gitleaks.sarif --exit-code 0 --no-banner`; always exits 0 |
| TruffleHog (verified-only, hard-fail) | `trufflehog filesystem "${SCAN_DIR}" --results=verified --json --no-update`; if any verified finding and `fail-on-verified: true`, `exit 1` |
| Upload Gitleaks SARIF | `github/codeql-action/upload-sarif` (`8aad20d150bbac5944a9f9d289da16a4b0d87c1e`, v4.36.2), category `gitleaks` |
| Upload secrets evidence artifact | `actions/upload-artifact` (`043fb46d1a93c77aae656e7c1c64a875d1fc6a0a`, v7.0.1), name `secrets-sarif`, path `gitleaks.sarif` |

## Failure behavior

| Tool | Failure mode |
| --- | --- |
| Gitleaks | Never fails the job directly (`--exit-code 0`). Findings land in the code-scanning SARIF hub as soft-fail signals. |
| TruffleHog | Fails the job (`exit 1`) only on a **verified** (confirmed-live) secret. Unverified candidate matches do not fail the job. |

## What each tool detects

| Tool | Detection mode |
| --- | --- |
| Gitleaks | Pattern-based (100+ built-in rules): API keys, tokens, private keys, connection strings |
| TruffleHog | Pattern match **plus live verification** against the credential's own provider API — only confirmed-live secrets count as "verified" |

## Local configuration file

`.gitleaks.toml` exists at the repository root (`useDefault = true`, plus an
allowlist for `docs/.*\.md` / `\.typos\.toml` paths and placeholder/SHA
regexes). The `gitleaks dir` invocation inside `reusable-secrets.yml` does not
pass a `--config` flag, so it is not confirmed from the reusable's source
whether `.gitleaks.toml` is picked up automatically (Gitleaks' own default
config-discovery behavior) or is unused by this workflow — verify with a local
run (see Examples) before relying on it.

## Examples

Reproduce the Gitleaks scan locally, including the local config file:

```bash
gitleaks dir . --config .gitleaks.toml --report-format sarif --report-path gitleaks.sarif --no-banner
```

Reproduce the TruffleHog verified-only scan locally:

```bash
trufflehog filesystem . --results=verified --json
```
