---
id: explanation-attested-delivery-end-to-end
type: semantic
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: explanation/security
title: Attested Delivery, End to End
tags:
  - explanation
  - security
  - attested-delivery
  - supply-chain
  - ci-cd
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
relationships:
  - type: relates-to
    target: SIGNED-RELEASES.md
  - type: relates-to
    target: ../runbooks/RELEASING.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Attested Delivery, End to End
  entity_type: explanation
---

# Attested Delivery, End to End

`mif-rs` is a **Mode-A consumer** of the central reusable-workflow repository
`modeled-information-format/.github`: it does not re-implement scanning, signing, or
verification logic — it calls central reusables pinned to a full commit SHA, and
each gate's verdict normalizes on SARIF. This document explains *why* the pipeline
is shaped the way it is: why evidence gets produced at two separate points rather
than one, why a `publish = false` switch exists per crate, and why a tag publishes
nothing that hasn't already been verified. For the verification commands
themselves, see [`SIGNED-RELEASES.md`](./SIGNED-RELEASES.md) and `SECURITY.md` §
Verifying Release Artifacts; for the operational sequence of cutting a release,
see [`RELEASING.md`](../runbooks/RELEASING.md).

## Why two seams, not one

Attested delivery has two distinct points where evidence is produced and bound to
a subject, and keeping them conceptually separate is the key to understanding the
whole system.

The first is **merge-time**: every push or pull request to `main` runs SAST
(CodeQL), SCA (OSV-Scanner), supply-chain posture (OpenSSF Scorecard), and an
IaC/license scan (Trivy) over the source tree. Each emits SARIF, uploaded to the
repository's code-scanning hub, and the *Code scanning results* required check is
what actually gates the merge. This is a **repository-level** signal — it says
"this commit, as a body of source, passed these scans" — and it is cheap to run
often.

The second is **deploy-time**: on a tag push (release binaries, crates.io) or a
push to `main` once external publishing is armed (the container image), a subset
of those same scanners re-runs, but this time their SARIF output is not merely
uploaded — it is signed as an in-toto attestation, keyless via Sigstore, and bound
by digest to a specific published artifact. This is an **artifact-level** claim:
"this exact byte sequence, identified by this digest, carries this verdict."

The reason these can't collapse into a single pass is that they answer different
questions for different audiences. A merge-time SARIF upload tells a reviewer
"don't merge this." A deploy-time attestation tells a consumer, potentially
downloading the binary months later on an unrelated machine, "this specific
artifact was scanned, and here is cryptographic proof of what the scan found." The
first is ephemeral CI state; the second has to outlive the CI run and be
independently checkable with `gh attestation verify`. Four central reusables power
the merge-time seam (`reusable-sast-codeql.yml`, `reusable-sca-osv.yml`,
`reusable-scorecard.yml`, `reusable-trivy.yml`, each called from
`quality-gates.yml`), and three of them re-run at deploy time in `release.yml` —
SAST, SCA, and IaC/license — because the artifact a consumer actually downloads
(a published source snapshot) needs its own bound verdict, not a promise that some
earlier, unrelated commit was once scanned clean. Supply-chain posture (Scorecard)
stays merge-time only, because it characterizes the *repository* as an ongoing
concern, not a single artifact.

## Why a `publish = false` switch, and why it lives per crate

Each of the five workspace members — `mif-core`, `mif-schema`, `mif-ontology`,
`mif-cli`, `mif-mcp` — carries `publish = false` in its own `crates/<name>/Cargo.toml`.
This is a deliberate holdover from the template this repository was forked from
(`attested-delivery/rust-template`), and it exists to decouple *build and attest*
from *publish externally*. The distinction matters because a GitHub Release is a
tag primitive, not an external publish: a pushed `v*.*.*` tag always produces an
attested GitHub Release — binaries, SBOM, and a source snapshot, each carrying
signed provenance — provided the fail-closed `verify` job passes, *regardless* of
whether any crate is publishable. What `publish = false` withholds is specifically
the **external distribution channels**: crates.io (`publish.yml`), the container
registry (`pipeline.yml`'s `docker` chain), and the Homebrew tap
(`package-homebrew.yml`). All three resolve `publishable` from `cargo metadata` at
runtime, so arming a channel is a one-line deletion in the relevant crate's
manifest rather than a workflow edit.

The rationale for keeping the evidence chain unconditional while gating
distribution is that attestation and distribution are separable concerns.
Verifiable provenance is worth generating on every tagged build — it costs a few
CI minutes and produces something auditable even for a crate nobody has decided to
ship publicly yet. Distribution, by contrast, is often irreversible (crates.io
cannot be unpublished; a container tag pushed to a public registry is effectively
permanent) and should require a conscious decision, not an accidental side effect
of tagging.

## Why quality-gate verdicts get re-signed at release time

The four merge-time reusables produce SARIF that is useful the moment it's
generated but stops being *provable* once the CI run that produced it ages out.
`release.yml` re-runs SAST, SCA, and IaC/license over a **published source
snapshot** — a `git archive` tarball attached to the release as a real,
downloadable asset — specifically so the resulting SARIF has something durable to
bind to. Each gate's output is then handed to the central
`reusable-attest-scan.yml`, which signs it as an in-toto attestation keyed to the
snapshot's digest, under a predicate type scoped to that gate (`.../sca/v1`,
`.../iac-license/v1`, `.../sast/v1`). A fourth signal, an OpenVEX vulnerability
disposition (`reusable-vex.yml`), is attested the same way, self-signed under its
own identity, so that the release gate can enforce "no undispositioned
high/critical finding" rather than the much blunter "zero findings" — a
distinction that matters because a real-world dependency graph almost always
carries *some* advisory that has already been triaged as not applicable.

The `verify` job that follows is deliberately positioned **before** the GitHub
Release exists, and it is fail-closed: it downloads every artifact, insists on
finding exactly the expected set (a partial set must never ship), and then
verifies provenance, SBOM, and every gate-verdict attestation against the source
snapshot before the `release` job is even allowed to run. This ordering is the
whole point — a release that fails verification is never created, rather than
being created and later revoked. `test` and `audit` (cargo-audit) sit as
dependencies of the `release` job for the same reason: a tag is *untrusted input*.
Nothing guarantees it points at a commit that ever passed CI, so the release
workflow re-derives that guarantee itself instead of trusting the tag.

## Why the container chain is a separate deploy path

`pipeline.yml`'s container jobs (`docker`, `docker-sign`, `docker-verify`,
`gate-image`, `attest-container-scan`) follow the same shape — build, sign,
verify, fail-closed — but with one structural difference: the signing identity is
the **central** `sign-and-attest.yml` workflow, not this repository's own. Under
SLSA Build L3, that separation is what makes the provenance non-falsifiable — if
this repository's own workflow could sign its own build claims, a compromised
workflow file could forge them. Routing signing through an isolated, centrally
owned workflow means the attestation asserts two independent things at once:
*where* the build ran (this repo, via `--repo`) and *who* signed it (the central
workflow, via `--signer-workflow`) — and a verifier has to check both, because
`--repo` alone is insufficient to catch a spoofed signer. The container path is
currently dormant: it only runs once a crate's `publish = false` is lifted, per
the switch described above.

## Why crates.io publishing uses OIDC, not a stored token

`publish.yml` authenticates to crates.io via `rust-lang/crates-io-auth-action`,
which exchanges the workflow's own OIDC identity for a short-lived registry
token — there is no long-lived `CARGO_REGISTRY_TOKEN` secret to leak, rotate, or
scope down. The workflow then does something that only makes sense once you take
seriously the idea that *what the registry actually serves* is the thing worth
attesting: after publishing, it downloads the `.crate` archive back from
`static.crates.io`, byte-compares it against the package it built locally (cargo
packaging is deterministic for a given commit, so any mismatch is a real problem,
not noise), and only then attaches SLSA provenance — to the downloaded bytes, not
to a local rebuild. A local build asserting its own correctness proves less than
independently re-fetching what a third party is now distributing and attesting
that.

## Why Homebrew propagation is decoupled from the release event

The GitHub Release in the flow above is authored by `github-actions[bot]`, and
bot-authored release events do not trigger other workflows — a quirk of GitHub
Actions, not a design choice. `package-homebrew.yml` works around this by
listening for `workflow_run` completion on the Release workflow itself (with the
native `release: published` event and manual dispatch as fallbacks), and only
proceeds for a run whose conclusion was success and whose branch name looks like a
tag. It re-resolves the crate's metadata **at the released tag**, not at whatever
commit happens to be checked out when the workflow fires, because the formula has
to describe the artifact that was actually released, not the latest state of
`main`. The formula push is written to be idempotent — a re-fire for a version
that produces no diff is a no-op — since `workflow_run` and `release` firing
together for the same event is expected, not a bug to guard against.

## Where this doesn't yet match the surrounding docs

Two details worth naming honestly, since an explanation that hides its own edges
isn't doing its job. First, the environment gate on `publish.yml`, `release.yml`,
and `package-homebrew.yml` is still named `copilot` — inherited verbatim from the
upstream template — rather than something specific to this repository; renaming
it to something like `release` and configuring real protection rules on it is
tracked as follow-on work, not yet done. Second, `release.yml`'s metadata
resolution step currently reads a single `[[bin]]` target from the first package
`cargo metadata` returns; extending it to build every binary crate in this
workspace (`mif-cli` and `mif-mcp`) is a separate, not-yet-landed change. Neither
gap affects the *shape* of the attestation architecture described above — it
affects how many binaries flow through it.

## In short

The pipeline splits evidence production into a cheap, repeated merge-time pass and
a rarer, digest-bound deploy-time pass because those two passes answer different
questions to different audiences. `publish = false` per crate keeps attestation
unconditional while making external distribution a deliberate, reversible-until-
irreversible decision. Re-signing gate verdicts over a published source snapshot,
verifying fail-closed before the release exists, authenticating to crates.io via
OIDC instead of a stored secret, and re-attesting the registry-served `.crate`
rather than a local build are all instances of the same underlying discipline:
prefer verifying the thing a consumer will actually receive over trusting an
earlier, indirect claim about it.
