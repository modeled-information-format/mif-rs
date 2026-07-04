---
id: how-to-deploy-mif-rs-release
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/deployment
title: How to Deploy a mif-rs Release
tags:
  - how-to
  - deployment
  - release
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
relationships:
  - type: relates-to
    target: runbooks/RELEASING.md
  - type: relates-to
    target: security/SIGNED-RELEASES.md
  - type: relates-to
    target: security/ATTESTED-DELIVERY.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Deploy a mif-rs Release
  entity_type: how-to-guide
---

# How to Deploy a mif-rs Release

Cut a tagged release of `mif-rs` and confirm it landed on every armed
distribution channel: GitHub Releases (multi-platform binaries for
`mif-cli`/`mif-mcp`), crates.io (every workspace crate — none carries
`publish = false`), and the container image on GHCR. For the full
pre-release checklist, monitoring detail, rollback, and hotfix procedures,
see [`RELEASING.md`](https://modeled-information-format.github.io/mif-rs/runbooks/releasing/); this guide covers the direct
path from a version bump to a verified, live release.

## Prerequisites

- Push access to `main` and permission to push tags.
- `gh` CLI, authenticated.
- crates.io Trusted Publishing configured once, per crate you intend to
  publish: on crates.io, crate **Settings → Trusted Publishing → Add**,
  repository `modeled-information-format/mif-rs`, workflow `publish.yml`,
  environment `release`. Publishing uses OIDC — no `CARGO_REGISTRY_TOKEN`
  secret exists.
- If you intend to push a container image: **Settings → Actions → General
  → Workflow permissions → "Read and write permissions"**, so
  `pipeline.yml` can push to GHCR.
- If you intend to update a Homebrew tap: secret `HOMEBREW_TAP_TOKEN` (a
  PAT with write access to `{owner}/homebrew-tap`) and, optionally, the
  `HOMEBREW_TAP_REPO` variable to override the tap repo name.

## Step 1 — Bump the version

Edit the single version field in the workspace root — every crate inherits
it via `version.workspace = true`:

```toml
# Cargo.toml
[workspace.package]
version = "0.1.1"  # Update this
```

## Step 2 — Run the local check suite

```bash
just check
```

<details>
<summary>Raw cargo equivalent</summary>

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
```

</details>

## Step 3 — Commit the version bump and tag

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.1.1"
git push

git tag -a v0.1.1 -m "Release v0.1.1"
git push origin v0.1.1
```

The tag push triggers `release.yml` (binaries + SBOM + attestations + the
GitHub Release), `publish.yml` (crates.io, for all 9 publishable workspace
members), and — on a non-PR push — `pipeline.yml`'s container chain.
`package-homebrew.yml` follows automatically once
`release.yml` completes. See
[`ATTESTED-DELIVERY.md`](https://modeled-information-format.github.io/mif-rs/security/attested-delivery/) for why the pipeline
is shaped this way, and [`SIGNED-RELEASES.md`](https://modeled-information-format.github.io/mif-rs/security/signed-releases/)
for what each attestation proves.

## Step 4 — Watch the run

```bash
gh run watch "$(gh run list --workflow=release.yml --limit=1 --json databaseId -q '.[0].databaseId')"
```

A green `Verify Attestations` job is the signal the release will actually be
created — that job runs fail-closed, before the GitHub Release exists.

## Step 5 — Verify each channel

**GitHub Release** — download and verify a binary:

```bash
gh release download v0.1.1 --repo modeled-information-format/mif-rs \
  --pattern 'mif-cli-0.1.1-linux-amd64'
gh attestation verify mif-cli-0.1.1-linux-amd64 \
  --repo modeled-information-format/mif-rs
```

**crates.io**:

```bash
cargo search mif-cli
```

**Docker/GHCR**:

```bash
docker pull ghcr.io/modeled-information-format/mif-rs:v0.1.1
docker run --rm ghcr.io/modeled-information-format/mif-rs:v0.1.1 --version
```

Full verification commands for every artifact type (provenance, SBOM,
checksums, container images, the published crate) are in
`SECURITY.md` § Verifying Release Artifacts.

## Done

The tag is pushed, the fail-closed verify gate passed, and the release is
live on every channel you checked in Step 5. For rollback, hotfixes, and the
full post-release checklist, continue with
[`RELEASING.md`](https://modeled-information-format.github.io/mif-rs/runbooks/releasing/).
