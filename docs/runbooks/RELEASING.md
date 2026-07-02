---
id: how-to-release-mif-rs
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/release
title: How to Release mif-rs
tags:
  - how-to
  - release
  - crates-io
  - attested-delivery
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
relationships:
  - type: relates-to
    target: docs/runbooks/CI-TROUBLESHOOTING.md
  - type: relates-to
    target: SECURITY.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Release mif-rs
  entity_type: how-to-guide
---

# How to Release mif-rs

Create, monitor, and — if needed — roll back a release of the `mif-rs`
workspace: 5 crates (`mif-core`, `mif-schema`, `mif-ontology`, `mif-cli`,
`mif-mcp`) published independently to crates.io, plus attested binaries for
the two binary crates (`mif-cli`, `mif-mcp`) and a container image.

> **Prefer the `/release` skill.** Releases are orchestrated end-to-end by
> the `/release` skill (`.github/skills/release/SKILL.md`): release-prep PR,
> tag, monitoring of every workflow chain, and independent workstation
> verification. The manual procedure below is the same process the skill
> drives.

## Version Numbering (SemVer)

All 5 crates share one workspace version (`version.workspace = true` in
every crate's `Cargo.toml`, set once in `[workspace.package]`). This project
follows [Semantic Versioning 2.0.0](https://semver.org/):

| Change type | Version bump | Example | When to use |
|---|---|---|---|
| Breaking API change in any published crate | **MAJOR** | `1.0.0` -> `2.0.0` | Removed public types, changed function signatures |
| New feature (backward-compatible) | **MINOR** | `0.1.0` -> `0.2.0` | New public functions, new optional fields |
| Bug fix (backward-compatible) | **PATCH** | `0.1.0` -> `0.1.1` | Fix incorrect behavior, performance improvement |

**Pre-1.0 policy:** while on `0.x.y`, MINOR bumps may include breaking
changes. Document these clearly in commit messages with `BREAKING CHANGE:`
in the body.

---

## Prerequisites

### Publication gate

Every crate currently ships with `publish = false` in its `[package]`
section — releasing requires deleting that line from each crate's
`Cargo.toml` before the first real release. Check the current state:

```bash
cargo metadata --no-deps --locked --format-version 1 \
  | jq -r '.packages[] | select(.publish != null) | .name' \
  | { grep -q . && echo "publish-disabled crates:" && cargo metadata --no-deps --locked --format-version 1 \
      | jq -r '.packages[] | select(.publish != null) | .name'; } || echo "all crates publishable"
```

### Required Secrets and Setup

Configure on crates.io and in GitHub repository settings (**Settings >
Secrets and variables > Actions**):

| Item | Purpose | How to set up |
|---|---|---|
| crates.io Trusted Publishing (one-time, **per crate**) | Publish via OIDC — no long-lived token | On crates.io, for each of the 5 crates: crate **Settings > Trusted Publishing** > add repo `modeled-information-format/mif-rs`, workflow `publish.yml`, environment `release` |
| `HOMEBREW_TAP_TOKEN` (secret, optional) | Push formula updates to the Homebrew tap | Fine-grained PAT with write access to `{owner}/homebrew-tap` |
| `HOMEBREW_TAP_REPO` (variable, optional) | Override the tap repo name (default `homebrew-tap`) | **Settings > Secrets and variables > Actions > Variables** |
| `GITHUB_TOKEN` | Provided automatically | No setup needed |

### Environment protection

`publish.yml`, `release.yml`, and `package-homebrew.yml` all gate on the
`release` GitHub Environment. Configure real protection rules (at minimum a
required reviewer) on it in **Settings > Environments** before arming
external publish channels.

### Permissions

- **GitHub Packages (Docker):** Settings > Actions > General > Workflow
  permissions > "Read and write permissions"

---

## Pre-Release Checklist

Run through this checklist before every release.

- [ ] All CI checks pass on `main` (check
      [Actions](https://github.com/modeled-information-format/mif-rs/actions))
- [ ] Update the workspace version in the root `Cargo.toml`:
  ```toml
  [workspace.package]
  version = "X.Y.Z"  # New version — every crate inherits it via version.workspace = true
  ```
- [ ] Run the full local check suite:
  ```bash
  just check    # fmt-check + lint + test + doc-build + deny
  just msrv     # cargo +1.92 check --all-features
  ```
- [ ] Build both release binaries locally to verify:
  ```bash
  cargo build --release -p mif-cli -p mif-mcp
  ```
- [ ] Review `CHANGELOG.md` and recent commits since the last tag:
  ```bash
  git log $(git describe --tags --abbrev=0)..HEAD --oneline
  ```
- [ ] Verify conventional commit messages are correct (they drive changelog
      generation)
- [ ] If breaking changes exist, confirm a MAJOR version bump and
      `BREAKING CHANGE:` in commit bodies
- [ ] Commit the version bump separately:
  ```bash
  cargo check   # regenerates Cargo.lock for the new version — never hand-edit it
  git add Cargo.toml Cargo.lock
  git commit -m "chore: bump version to X.Y.Z"
  git push
  ```

---

## Step-by-Step: Create and Push a Release Tag

### 1. Create an Annotated Tag

```bash
git tag -a vX.Y.Z -m "Release vX.Y.Z"
```

### 2. Push the Tag

```bash
git push origin vX.Y.Z
```

This single push triggers all release automation.

### 3. Triggered Workflows

Pushing a `v*.*.*` tag triggers these workflows in parallel:

| Workflow | File | What it does |
|---|---|---|
| **Release** | `release.yml` | Builds `mif-cli` and `mif-mcp` across 5 platforms each (`{bin}-{version}-{platform}`) with SLSA build provenance, generates + attests a CycloneDX SBOM, verifies every attestation **fail-closed**, then creates the GitHub Release with auto-generated notes |
| **Publish** | `publish.yml` | Publishes `mif-core`, `mif-schema`, `mif-ontology`, `mif-cli`, and `mif-mcp` to crates.io independently, in dependency order, each via its own crates.io Trusted Publishing (OIDC) config — then downloads each registry-served `.crate`, byte-compares it, and attests it |
| **Pipeline (container)** | `pipeline.yml` | Builds and pushes the multi-platform container image (linux/amd64, linux/arm64) to `ghcr.io/modeled-information-format/mif-rs`, signed/attested by the central signer workflow and verified fail-closed |

After the Release workflow completes, `package-homebrew.yml` fires via
`workflow_run` and regenerates the tap formula(e) in `{owner}/homebrew-tap`.

**Never re-run `release.yml` against an existing tag.** Builds are not
reproducible; a re-run would overwrite published assets with different
bytes, violating the immutability the attestations protect.

---

## Monitoring Workflow Progress

### GitHub Actions Dashboard

- **All workflows:** https://github.com/modeled-information-format/mif-rs/actions
- **Filter by tag:** click the specific workflow run triggered by the tag push

### CLI Monitoring

```bash
gh run list --limit 10
gh run watch <run-id>
gh run view <run-id> --log-failed
```

### What to Watch For

| Stage | Expected duration | Common failure point |
|---|---|---|
| Build Binaries (2 bins x 5 platforms) | ~10-15 min | The macos-amd64 leg (cross-target on `macos-latest`), for either binary |
| Test + Cargo Audit gates | ~3 min | New advisory in `Cargo.lock` (audit scans the raw lockfile; deny may not have flagged it) |
| SBOM (generate + attest) | ~1 min | Attestation permissions (`id-token`, `attestations`) |
| Verify Attestations | &lt;1 min | Fail-closed gate: any missing/unverifiable attestation blocks the release |
| Create Release | ~1 min | Only runs on tags, after verify passes |
| Publish (crates.io, 5 crates) | ~5 min | Trusted Publishing not configured for one of the 5 crates; dependency-order failure blocking a downstream crate |
| Docker chain (pipeline) | ~5-10 min | Buildx multi-platform, central signer/verify |
| Homebrew (after Release) | ~2 min | `workflow_run` trigger, tap token |

---

## Post-Release Verification

Run through this after all workflows complete.

- [ ] **GitHub Release** exists with correct version:
  ```bash
  gh release view vX.Y.Z
  ```
- [ ] **Binary assets** are attached for both binary crates, on every
      platform (`{bin}-{version}-{platform}` per binary — `mif-cli` and
      `mif-mcp` each across linux-amd64, linux-arm64, macos-amd64,
      macos-arm64, windows-amd64.exe), plus SBOM and checksum assets:
  ```bash
  gh release view vX.Y.Z --json assets --jq '.assets[].name'
  ```
- [ ] **Attestations verify** from an independent machine (full reference:
      [SECURITY.md](../../SECURITY.md#verifying-release-artifacts)):
  ```bash
  gh release download vX.Y.Z --repo modeled-information-format/mif-rs
  for BIN in mif-cli mif-mcp; do
    gh attestation verify "${BIN}-X.Y.Z-linux-amd64" \
      --repo modeled-information-format/mif-rs
    gh attestation verify "${BIN}-X.Y.Z-linux-amd64" \
      --repo modeled-information-format/mif-rs \
      --predicate-type https://cyclonedx.org/bom
  done
  shasum -a 256 -c *-X.Y.Z-checksums.txt
  ```
- [ ] **Release notes** are generated correctly
- [ ] **Container image** is pushed:
  ```bash
  docker pull ghcr.io/modeled-information-format/mif-rs:vX.Y.Z
  docker pull ghcr.io/modeled-information-format/mif-rs:latest
  ```
- [ ] **crates.io** — each of the 5 crates is updated, and each served
      `.crate` attestation verifies:
  ```bash
  for NAME in mif-core mif-schema mif-ontology mif-cli mif-mcp; do
    curl -fsSL -A 'release-check' \
      -O "https://static.crates.io/crates/${NAME}/${NAME}-X.Y.Z.crate"
    gh attestation verify "${NAME}-X.Y.Z.crate" \
      --repo modeled-information-format/mif-rs
  done
  ```
- [ ] **Homebrew formula** updated in the tap (a `package-homebrew.yml` run
      appeared after Release completed)
- [ ] Install and test each binary crate on at least one platform:
  ```bash
  cargo install mif-cli --locked
  cargo install mif-mcp --locked
  mif-cli --version
  mif-mcp --version
  ```

---

## Rollback Procedures

### Roll Back a GitHub Release

```bash
gh release delete vX.Y.Z --yes
git push --delete origin vX.Y.Z
git tag -d vX.Y.Z
```

### Roll Back a crates.io Publish

**You cannot unpublish from crates.io.** Your options, per affected crate:

1. **Yank the version** (prevents new projects from depending on it — this
   is a workspace with multiple published crates, so `-p` is required):
   ```bash
   cargo yank --version X.Y.Z -p <crate-name>
   ```
2. **Publish a fix** as a patch release, bumping the shared workspace
   version (see [Hotfix Release Process](#hotfix-release-process) below).

### Roll Back the Container Image

GHCR images are immutable by tag. To mitigate:

1. **Point users to a previous version:**
   ```bash
   docker pull ghcr.io/modeled-information-format/mif-rs:vPREVIOUS
   ```
2. **Delete the package version** via GitHub UI: Packages > mif-rs > Package
   versions > Delete.
3. **Re-tag `latest`** to the previous good version by re-pushing a
   known-good tag.

---

## Hotfix Release Process

When a critical bug or security issue is found in the latest release:

### 1. Create a Hotfix Branch

```bash
git checkout -b hotfix/vX.Y.(Z+1) vX.Y.Z
```

### 2. Apply the Fix

```bash
just check
just msrv
```

### 3. Bump Version and Tag

The workspace version lives in one place, so a hotfix only touches
`[workspace.package].version` in the root `Cargo.toml`:

```bash
# Edit Cargo.toml: version = "X.Y.(Z+1)"
cargo check   # regenerates Cargo.lock
git add -A
git commit -m "fix: <description of the critical fix>"
```

### 4. Merge and Release

```bash
git checkout main
git merge hotfix/vX.Y.(Z+1)
git push origin main

git tag -a vX.Y.(Z+1) -m "Release vX.Y.(Z+1)"
git push origin vX.Y.(Z+1)
```

### 5. If the Bad Version Was on crates.io

```bash
# Yank the bad version from each affected crate
cargo yank --version X.Y.Z -p <crate-name>
# The hotfix tag push triggers an automatic re-publish of X.Y.(Z+1) for
# every crate, not just the one that changed — all 5 share one version.
```

---

## Changelog and Release Notes

`CHANGELOG.md` (workspace root, covering all 5 crates under the shared
version) is maintained by hand (Keep a Changelog format) and updated
**before** tagging: the release-prep step moves the `## [Unreleased]`
entries under a new `## [X.Y.Z] - <date>` heading and updates the compare
links (the `/release` skill does this in the prep PR). GitHub Release notes
are auto-generated by the Release workflow (`generate_release_notes: true`).

Conventional commit prefixes map onto changelog sections:

| Commit prefix | Changelog section |
|---|---|
| `feat:` | Added |
| `fix:` | Fixed |
| `docs:` | Documentation |
| `perf:` | Performance |
| `refactor:` | Refactored |
| `test:` | Testing |
| `chore:` | Miscellaneous |

**Best practices:**
- Use scoped prefixes for clarity: `feat(mif-schema): add citation validator`
- Include `BREAKING CHANGE:` in the commit body for breaking changes
- A release with an empty `[Unreleased]` section is a red flag — stop and
  confirm what the release contains

---

## Deployment Targets Quick Reference

### GitHub Releases

- **URL:** https://github.com/modeled-information-format/mif-rs/releases
- **Binaries:** `mif-cli`, `mif-mcp`
- **Platforms:** Linux (amd64, arm64), macOS (amd64, arm64), Windows (amd64)
- **Attestations:** SLSA build provenance + CycloneDX SBOM attestation per
  binary; verify per
  [SECURITY.md](../../SECURITY.md#verifying-release-artifacts)

### Container Image (GHCR)

- **Registry:** `ghcr.io/modeled-information-format/mif-rs`
- **Platforms:** linux/amd64, linux/arm64
- **Base image:** `distroless/cc-debian12` (minimal attack surface)
- **User:** `nonroot:nonroot` (unprivileged)
- **Tags:** `vX.Y.Z`, `X.Y`, `X`, `latest`, `sha-<commit>`

### crates.io

- **Packages:** `mif-core`, `mif-schema`, `mif-ontology`, `mif-cli`,
  `mif-mcp` — each published independently, each requiring its own one-time
  Trusted Publishing setup (see Prerequisites above). A failure on one crate
  does not block the others' channels, but does block anything published
  after it in the dependency order.

### Homebrew Tap

- **Tap:** `{owner}/homebrew-tap` (override with the `HOMEBREW_TAP_REPO`
  variable)
- **Formula:** generated from Cargo.toml metadata after each release

### Install Methods

```bash
# From GitHub release (Linux)
wget https://github.com/modeled-information-format/mif-rs/releases/download/vX.Y.Z/mif-cli-X.Y.Z-linux-amd64
chmod +x mif-cli-X.Y.Z-linux-amd64

# From crates.io
cargo install mif-cli
cargo install mif-mcp

# From source
cargo install --git https://github.com/modeled-information-format/mif-rs mif-cli
```

---

## Troubleshooting

See [CI-TROUBLESHOOTING.md](CI-TROUBLESHOOTING.md) § Release workflow
failures and § Docker build failures for the full table of failure modes and
fixes. The release process is complete once every checkbox in Post-Release
Verification passes.
