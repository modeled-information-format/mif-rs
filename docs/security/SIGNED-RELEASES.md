---
id: explanation-signed-releases-and-slsa-provenance
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: explanation/security
title: Signed Releases & SLSA Provenance
tags:
  - explanation
  - security
  - slsa
  - sigstore
  - supply-chain
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
provenance:
  '@type': Provenance
  sourceType: user_explicit
  trustLevel: high_confidence
  wasAttributedTo:
    '@id': urn:mif:team:mif-rs-maintainers
    '@type': prov:Agent
citations:
  - '@type': Citation
    citationType: specification
    citationRole: source
    title: SLSA — Supply-chain Levels for Software Artifacts, v1.0
    url: https://slsa.dev/spec/v1.0/
    accessed: '2026-07-02'
  - '@type': Citation
    citationType: specification
    citationRole: source
    title: Sigstore
    url: https://www.sigstore.dev/
    accessed: '2026-07-02'
relationships:
  - type: relates-to
    target: ATTESTED-DELIVERY.md
  - type: relates-to
    target: ../../SECURITY.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Signed Releases & SLSA Provenance
  entity_type: explanation
---

# Signed Releases & SLSA Provenance

Every release artifact this project produces carries a cryptographic
attestation, and a fail-closed verification job runs **before** the GitHub
Release exists — nothing publishes unverified. This document explains why
attestation is worth the CI cost, why the mechanism is keyless Sigstore signing
rather than a managed key, and why a few specific design choices (re-attesting
the registry-served `.crate`, signing container images under a separate
identity) look the way they do. It does not repeat verification commands —
those live in `SECURITY.md` § Verifying Release Artifacts, kept as the single
canonical, copy-pasteable source — and it does not re-walk the whole pipeline
job-by-job, which [`ATTESTED-DELIVERY.md`](./ATTESTED-DELIVERY.md) already
covers.

## Why attest at all

An artifact sitting in a release page makes three claims implicitly: that it was
built from the source it claims to be built from, that it hasn't been altered
since, and that it's the same bytes everyone else downloading it also gets.
Without attestation, all three are just assertions a consumer has to take on
trust. Attestation turns them into something checkable: **authenticity** (this
was built by this repository's workflow, not substituted by someone with write
access to a release page), **integrity** (the digest a verifier computes matches
the one the signer attested to, so tampering or corruption is detectable), and
**non-repudiation** (the attestation binds the artifact to the exact commit,
workflow, and run that produced it, so the claim can't quietly be walked back
later). None of this replaces good engineering practice — it's what lets a
downstream consumer, who has no relationship with this project beyond a
download link, verify those claims for themselves instead of trusting a badge.

## Why GitHub Artifact Attestations for release binaries

Release binaries are attested with `actions/attest-build-provenance` at build
time and `actions/attest-sbom` once a CycloneDX SBOM has been generated,
binding both to the exact bytes staged for each platform. A dedicated,
fail-closed job then re-verifies every attestation before the `release` job is
allowed to run — so a broken attestation blocks the release outright, rather
than shipping and being caught later. This ordering only works because the
verification step and the release-creation step are separate jobs with a hard
dependency between them: the pipeline cannot accidentally race ahead and
publish first.

## Why keyless signing over a managed key

Every attestation in this project is signed via Sigstore's keyless flow: the
signer authenticates with the workflow's own OIDC identity, gets a short-lived
certificate from the Fulcio CA, and the resulting signature is logged to the
public Rekor transparency log. The alternative — a long-lived private signing
key stored as a repository secret — creates an asset that has to be generated,
scoped, rotated, and protected against exfiltration for as long as it exists,
and every signature it produces is only as trustworthy as that secret's custody
chain. Keyless signing sidesteps the whole problem: there is no persistent key
to compromise, because the certificate is minted fresh per signing event and
expires almost immediately. What a verifier checks instead is the *identity*
that requested the certificate (which workflow, on which repository, on which
run) — an assertion GitHub's OIDC issuer makes, not one this project's own
secrets could forge even if they leaked. The Rekor log exists so that even if a
certificate authority were somehow compromised, every signature it ever issued
remains publicly auditable after the fact.

## Why the published `.crate` gets re-attested, not the local build

`publish.yml` doesn't attest the archive it packaged locally. It publishes to
crates.io via Trusted Publishing (OIDC — no `CARGO_REGISTRY_TOKEN` secret
exists to leak), then downloads the `.crate` that `static.crates.io` actually
serves, byte-compares it against the local package, and only *then* attaches
SLSA provenance to those downloaded bytes. The distinction matters because
what a `cargo install` actually fetches is whatever the registry hands back —
not the archive that happened to sit in this CI run's workspace. Attesting the
local build would prove something true but slightly beside the point; attesting
the registry-served bytes proves the thing a consumer will actually receive.
The byte-comparison step exists because cargo's packaging is deterministic for
a given commit — if the registry ever serves something that doesn't match,
that's a real problem worth failing loudly on, not a discrepancy to paper over.

## Why container images are signed by a central identity, not this repository

Container images are not signed by `mif-rs`'s own workflow. They're signed and
attested by the centralized `modeled-information-format/.github` signer
workflow (`sign-and-attest.yml`), then verified fail-closed by this
repository's `docker-verify` job. This is the one place the attestation
architecture deliberately gives up local control in exchange for a stronger
guarantee: under SLSA Build L3, provenance is only non-falsifiable if the
entity making the claim is isolated from the entity being claimed about. If
this repository's own workflow signed its own container builds, a compromised
workflow file in this repo could forge a clean provenance statement for a
tampered image — the signer and the thing being signed would share a trust
boundary. Routing signing through a workflow this repository doesn't control
breaks that boundary: verification has to check *both* where the build ran
(`--repo`, asserting this repository) and *who* signed it
(`--signer-workflow`, asserting the central workflow's identity) — `--repo`
alone is insufficient by design, because it can't rule out a spoofed signer.
The central signer also attaches an SBOM and a vulnerability-report
attestation as OCI referrers, following the same keyless mechanism as
everything else.

## SLSA levels, and where this pipeline sits

SLSA (Supply-chain Levels for Software Artifacts) grades build integrity from
documentation-only (Level 1) through hermetic, fully reproducible builds
(Level 4). Release binaries and the published crate are attested by this
repository's own workflow at roughly Level 2/3 — version-controlled, built on
a hosted build service, with non-falsifiable provenance for the parts that go
through the central seam. Container images specifically claim **Build L3**,
because that's the one artifact type where the signer identity is fully
isolated from the repository that triggered the build — the isolation
described above is precisely what SLSA L3 requires and what distinguishes it
from L2.

## What this deliberately doesn't cover

This document stops at *why* the attestation architecture is shaped this way.
For the exact `gh attestation verify` / `cosign verify` invocations, predicate
types, and troubleshooting steps, `SECURITY.md` § Verifying Release Artifacts
is the canonical, kept-current reference — duplicating those commands here
would just create a second copy to drift out of sync. For how every gate in
the pipeline (SAST, SCA, IaC/license, VEX) fits together stage by stage, see
[`ATTESTED-DELIVERY.md`](./ATTESTED-DELIVERY.md).
