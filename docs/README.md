---
id: reference-mif-rs-docs-index
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: reference/documentation
title: mif-rs Documentation Index
tags:
  - reference
  - documentation
  - index
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
provenance:
  '@type': Provenance
  sourceType: user_explicit
  trustLevel: verified
  wasDerivedFrom:
    '@id': urn:mif:tree:mif-rs/docs
    '@type': prov:Entity
citations:
  - '@type': Citation
    citationType: specification
    citationRole: methodology
    title: Diátaxis — Reference
    url: https://diataxis.fr/reference/
    accessed: '2026-07-02'
relationships:
  - type: relates-to
    target: adr/0002-documentation-directory-structure.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: mif-rs Documentation Index
  entity_type: documentation-index
---

# mif-rs Documentation Index

Catalog of every document under `docs/` in the `mif-rs` repository, organized
by topic subdirectory.

## Synopsis

```text
docs/
├── README.md            (this document)
├── DEPLOYMENT.md
├── adr/
├── distribution/
├── runbooks/
├── security/
├── testing/
├── ux/
└── workflows/
```

## Runbooks

Operational procedures under `docs/runbooks/`.

| Document | Diátaxis mode | Description |
| --- | --- | --- |
| [CI Troubleshooting](runbooks/CI-TROUBLESHOOTING.md) | how-to | Common CI failure patterns and fixes for mif-rs. |
| [Dependency Updates](runbooks/DEPENDENCY-UPDATES.md) | how-to | Managing Cargo and GitHub Actions dependencies. |
| [Releasing](runbooks/RELEASING.md) | how-to | End-to-end procedure for creating, monitoring, and rolling back releases. |
| [Security Incident Response](runbooks/SECURITY-RESPONSE.md) | how-to | Handling reported security vulnerabilities. |

## Security

Security architecture documents under `docs/security/`.

| Document | Diátaxis mode | Description |
| --- | --- | --- |
| [Attested Delivery, End to End](security/ATTESTED-DELIVERY.md) | explanation | How a change travels from a pull request to a signed, independently verifiable release, and which gate signs what. |
| [Signed Releases & SLSA Provenance](security/SIGNED-RELEASES.md) | explanation | Release artifact cryptographic attestations and the fail-closed verification gate. |

## Distribution

Publishing-channel documents under `docs/distribution/`.

| Document | Diátaxis mode | Description |
| --- | --- | --- |
| [Package Manager Distribution](distribution/PACKAGE-MANAGERS.md) | how-to | Distribution to Homebrew, Snap, and system package managers. |
| [Docker Multi-Registry Distribution](distribution/DOCKER-REGISTRIES.md) | reference | Automated Docker image publication to multiple container registries. |
| [Alternative Cargo Registries](distribution/ALTERNATIVE-REGISTRIES.md) | reference | Publishing to registries beyond crates.io. |

## Testing

Test-methodology documents under `docs/testing/`.

| Document | Diátaxis mode | Description |
| --- | --- | --- |
| [Property-Based Testing Guide](testing/PROPERTY-BASED-TESTING.md) | tutorial | Validating code properties across all inputs with proptest and quickcheck. |

## UX

CLI user-experience documents under `docs/ux/`.

| Document | Diátaxis mode | Description |
| --- | --- | --- |
| [Shell Completions](ux/SHELL-COMPLETIONS.md) | how-to | Generating shell completions with clap_complete. |
| [Man Pages Generation](ux/MAN-PAGES.md) | how-to | Generating Unix manual pages with clap_mangen. |

## Workflows

CI workflow documents under `docs/workflows/`.

| Document | Diátaxis mode | Description |
| --- | --- | --- |
| [Code Coverage Tracking](workflows/COVERAGE.md) | how-to | Coverage measurement with cargo-llvm-cov, with optional Codecov reporting. |
| [Code Quality Metrics](workflows/CODE-QUALITY.md) | how-to | Unsafe code detection, binary size analysis, and documentation coverage. |
| [Secrets Scanning with Gitleaks](workflows/SECRETS-SCAN.md) | how-to | Secret detection; fails CI when a secret is found. |
| [Container Vulnerability Scanning with Trivy](workflows/CONTAINER-SCAN.md) | how-to | Docker image scanning, with findings in the GitHub Security tab. |
| [Software Bill of Materials (SBOM)](workflows/SBOM.md) | how-to | CycloneDX SBOM generation for supply-chain transparency. |
| [Spell Checking with typos](workflows/SPELL-CHECK.md) | how-to | Spell checking of docs, code comments, and string literals; warns but does not fail CI. |

## Deployment

| Document | Diátaxis mode | Description |
| --- | --- | --- |
| [Deployment Guide](DEPLOYMENT.md) | how-to | Comprehensive deployment instructions for mif-rs. |

## Architectural Decision Records

Decision records under `docs/adr/`. See
[docs/adr/README.md](adr/README.md) for the full ADR lifecycle process.

| ADR | Title |
| --- | --- |
| [ADR-0001](adr/0001-use-architectural-decision-records.md) | Use Architectural Decision Records |
| [ADR-0002](adr/0002-documentation-directory-structure.md) | Documentation Directory Structure |
