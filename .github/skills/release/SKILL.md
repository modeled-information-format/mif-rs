---
name: release
argument-hint: v<X.Y.Z> | patch | minor | major
description: >-
  Orchestrate and monitor a full attested release of this project
  end-to-end: release-prep PR, tag, attested binaries + SBOM (per
  binary crate), crates.io Trusted Publishing with .crate attestation
  (per library/binary crate), container images + Homebrew propagation
  (per binary crate), and independent workstation verification. Use
  this skill whenever the user invokes /release v<n.n.n> or /release
  patch|minor|major, or says "cut a release", "ship a release",
  "release version X", "bump and release", "do a patch/minor/major
  release", "tag a new version", or anything else that means
  publishing a new version of this project. Do not improvise the
  release process from memory — this skill encodes hard-won fixes for
  failure modes that are invisible until a release breaks.
---

# Release Orchestration

Run a complete attested release for this repository. The argument is
either an explicit version (`/release v1.4.0`) or a bump type
(`/release patch|minor|major`), which computes the target from the
current `[workspace.package].version`. Every phase below ends with a
verification; do not proceed past a failure — fix it or stop and report.

This is a **9-crate workspace** — `mif-core`, `mif-problem`, `mif-schema`,
`mif-frontmatter`, `mif-ontology`, `mif-embed`, `mif-store` (libraries),
`mif-cli`, `mif-mcp` (binaries) — with **one shared version**
(`version.workspace = true` on every member). A release ships every
publishable crate together, at the same version number; there is
no per-crate version skew. Names are never hardcoded: resolve the full
package list from `cargo metadata --no-deps --format-version 1` (every
`.packages[]`, never `.packages[0]`) and the `owner/repo` from
`gh repo view --json nameWithOwner` at the start — this is what keeps the
crate count in this skill from going stale the next time a crate is added.

The pipeline this skill drives (all already wired in `.github/workflows/`):

```
prep PR ──merge──> tag push ──┬─> release.yml  (test + audit gates → bin x platform
                              │    matrix, 2 bins x 5 platforms → provenance + SBOM
                              │    attestations → fail-closed verify → GitHub Release)
                              ├─> publish.yml  (pre-publish checks → crates.io
                              │    Trusted Publishing → download each registry
                              │    .crate → sha256 match → attest all)
                              └─> pipeline.yml (container: build x2 (mif-cli,
                                   mif-mcp) → central sign-and-attest x2 →
                                   fail-closed verify x2)
release.yml completion ─workflow_run─> package-homebrew.yml (formula update x2)
```

## Help / no argument

If invoked with no argument, `--help`, or `help`, print this and stop —
do not start a release:

```
/release — attested release orchestration

USAGE
    /release v<X.Y.Z>     release an explicit version (e.g. /release v1.4.0)
    /release patch        bump X.Y.Z -> X.Y.(Z+1) and release
    /release minor        bump X.Y.Z -> X.(Y+1).0 and release
    /release major        bump X.Y.Z -> (X+1).0.0 and release

WHAT IT DOES
    prep PR (version locations) -> required-checks green -> squash merge
    -> annotated tag -> monitors: attested binaries (mif-cli, mif-mcp x 5
    platforms) + SBOM + fail-closed verify -> GitHub Release; crates.io
    Trusted Publishing + .crate attestation for every publishable crate; attested
    container images x2; Homebrew auto-update x2 -> independent
    workstation verification of every artifact.

NOTES
    - Publishing to crates.io is irreversible; versions are immutable.
    - CHANGELOG [Unreleased] must have content; empty means stop and ask.
    - Never re-runs release.yml against an existing tag (asset immutability).
```

## Phase 0 — Preflight

0. **Publication gate** — this project ships from `mif-rs`, where each
   of the 9 crates' publication is controlled independently by its own
   `publish` line in `crates/<name>/Cargo.toml` (the workflows read this
   via `cargo metadata`; a crate with `publish = false` is excluded from
   `cargo publish --workspace` and from Homebrew updates). This is
   separate from `pipeline.yml`'s `has-bin-target` gate, which is a single
   workspace-wide boolean — true whenever *any* member has a `[[bin]]`
   target — and controls whether the container chain runs at all; it does
   not exclude individual crates by their publish status, and a bin
   crate's GitHub-release binary is built regardless of that crate's
   `publish` line. Check the current publish state:
   ```bash
   cargo metadata --no-deps --format-version 1 \
     | jq -c '[.packages[] | {name, publishable: (.publish != [])}]'
   ```
   If a crate you expect to ship shows `publishable: false`, stop and
   confirm with the user whether that's intentional (e.g. arming
   `mif-cli`/`mif-mcp` before the libraries have their own crates.io
   history is a valid sequencing choice, not necessarily a mistake).
1. `git checkout main && git pull`; working tree must be clean of tracked
   changes (untracked noise is fine).
2. Resolve the target version from the argument:
   - `v<major>.<minor>.<patch>` → use as given.
   - `patch` | `minor` | `major` → read `[workspace.package].version` from
     the root `Cargo.toml` on freshly-pulled main and bump that component,
     zeroing the lower ones (`1.3.1` + `minor` → `1.4.0`; `1.3.1` + `major`
     → `2.0.0`).
   - Anything else → stop and ask.
   Strip the `v` for file edits; keep it for the tag. State the resolved
   version in the first progress message so a wrong bump is caught early.
3. Sanity checks, all hard stops:
   - New version is greater than `[workspace.package].version` in `Cargo.toml`.
   - Tag does not already exist (`git tag -l v<X.Y.Z>`, and check remote).
   - `CHANGELOG.md` has content under `## [Unreleased]` — a release with
     an empty changelog means something is off; ask the user what this
     release contains.
   - Latest pipeline run on main is green
     (`gh run list --workflow=pipeline.yml --branch main --limit 1`).
4. Semver gut-check: if Unreleased contains breaking changes or an MSRV
   bump and the user asked for a patch, raise it before proceeding.

## Phase 1 — Release prep

Branch `release/v<X.Y.Z>` off main, then update **all** version
locations (missing one ships inconsistent metadata):

| File | What to change |
| --- | --- |
| `Cargo.toml` | `[workspace.package].version = "<X.Y.Z>"` — one line arms every crate, since every member uses `version.workspace = true` |
| `Cargo.lock` | run `cargo check --workspace` after the Toml edit — never hand-edit |
| `CHANGELOG.md` | insert `## [<X.Y.Z>] - <today>` under `## [Unreleased]`; update the `[Unreleased]:` compare link and add the new version's compare link at the bottom |
| `SECURITY.md` | any `<bin>-<version>-<platform>` example versions, for both `mif-cli` and `mif-mcp` |
| `CITATION.cff` | if present: `version:` and `date-released:` — every occurrence |

Validate locally before the PR: `cargo fmt --all -- --check` and
`cargo check --workspace` minimum. The PR's CI and the release
workflow's own gates run the full suite, so don't duplicate the entire
gauntlet here — but a broken lockfile or fmt failure should never reach
the PR.

Commit as `chore(release): v<X.Y.Z>`, push, open the PR. The body should
list what the release contains and note anything operator-relevant.

## Phase 2 — PR through merge

Required status checks on main are `CI Checks / All Checks Pass` and
`pin-check / pin-check`. Monitor the PR with the Monitor tool — and use
the aggregate-gate guard, not a naive all-non-pending check:

```bash
# Terminal only when the aggregate gate check itself has reported and
# nothing is pending. Right after a push there is a window where only 1-2
# checks are registered; "zero pending" alone declares victory in that
# window (this produced a false ALL GREEN once).
gate=$(jq -r '[.[] | select(.name=="CI Checks / All Checks Pass")][0].bucket // "absent"' <<<"$checks")
```

When green, merge with `gh pr merge --squash --delete-branch`, then
`git checkout main && git pull` and confirm HEAD is the release commit.

## Phase 3 — Tag

Annotated tag on the merge commit, then push:

```bash
git tag -a v<X.Y.Z> -m "Release v<X.Y.Z>

<one-paragraph summary from the changelog>" <merge-sha>
git push origin v<X.Y.Z>
```

The tag push is the release trigger. Facts that matter here:

- Tag pushes bypass branch protection — release.yml carries its own test
  and audit gates precisely because the tag is untrusted input.
- **Never re-dispatch release.yml against an existing tag.** Builds are
  not reproducible; it would overwrite published release assets with
  different bytes, violating the immutability the attestations exist to
  protect.

## Phase 4 — Monitor the chains

Four things run; watch all of them with the Monitor tool (one monitor,
multiple conditions — report each as it lands):

1. **Release run** (`release.yml`). Expect: Resolve Project Metadata,
   Test, Cargo Audit, 10 × Build (`mif-cli` and `mif-mcp`, each across 5
   platforms), SBOM (generate + attest, one combined SBOM covering every
   binary), Verify Attestations, Create Release — all success.
2. **Publish run** (`publish.yml`). Expect: pre-publish checks, Trusted
   Publishing auth, a "Resolve unpublished members" step naming which of
   the 9 crates are actually being published this run (already-live
   versions are skipped, not an error), `cargo publish`, then the
   crate-attestation steps ("Download published crates from registry"
   and "Attest crate provenance" — both now loop over every publishable
   crate, not one). Report these step conclusions explicitly, and which
   crates were skipped as already-published if this is a re-run.
3. **Pipeline run** (`pipeline.yml`, container chain on the tag).
   Expect: Docker build/push **x2** (`mif-cli`, `mif-mcp`), Sign and
   Attest Image **x2** (central signer, matrixed per bin), Verify Image
   Attestations **x2** — all success. The Trivy image-scan gate
   (`gate-image`/`attest-container-scan`) runs **once**, against the
   first resolved bin only — this is deliberate (the org's
   `reusable-trivy.yml` uploads its SARIF under a fixed artifact name, so
   a second matrix cell in the same run would collide; both images share
   the same base image and dependency tree, so one scan is representative)
   — do not treat a missing second Trivy run as a bug.
4. **Homebrew run** (`package-homebrew.yml`) must appear **on its own**
   after the Release run completes, via `workflow_run`, and matrix over
   both bins (one formula update per bin). If no run appears within a few
   minutes of Release success, the trigger regressed — fall back to
   manual dispatch
   (`gh workflow run package-homebrew.yml -f version=<X.Y.Z> -f dry_run=false`)
   and investigate.

### Failure playbook

| Symptom | Cause | Action |
| --- | --- | --- |
| Publish auth fails: "No Trusted Publishing config found" | crates.io TP not configured for that specific crate | One-time setup on crates.io, **per crate**: crate → Settings → Trusted Publishing → repo `<owner>/<repo>`, workflow filename `publish.yml`, environment `release` (not `copilot` — that was the upstream template's name before this repo renamed it). Then `gh workflow run publish.yml --ref v<X.Y.Z>` (dispatch-on-tag makes `github.ref` the tag, so the tag-gated steps run). |
| Publish fails: "crate <name>@X.Y.Z already exists" | Duplicate publish attempt raced a successful one, or a re-run after a partial multi-crate failure | Benign for the crate(s) it names — `publish.yml`'s "Resolve unpublished members" step should have already skipped these; if it didn't, verify the version is live (`cargo search <name>`), report, move on. crates.io versions are immutable. |
| Crate download step exhausts retries | static.crates.io CDN propagation | Re-run the failed job; the publish itself succeeded. The step re-checks every publishable crate, so a re-run doesn't re-publish anything already live. |
| Crate sha256 mismatch (registry vs local package) for any crate | Should never happen — cargo packaging is deterministic per commit | Hard stop. Do not attest. Investigate before anything else. Report which specific crate(s) mismatched. |
| Cargo Audit job fails | Real advisory in `Cargo.lock` | Fix the dependency (usually `cargo update -p <crate>`) via a normal PR, then start the release over at Phase 0. Note: cargo-deny may NOT have flagged it — deny analyzes the feature/target graph, audit scans the raw lockfile; an unreachable phantom lock entry trips audit only. Both gates are intentional; keep both. |
| A build leg fails | Platform/toolchain issue for a specific (bin, platform) cell | The matrix is `mif-cli`/`mif-mcp` x 5 platforms: linux-amd64, linux-arm64 (`ubuntu-24.04-arm`), macos-arm64, macos-amd64 (cross-target on macos-latest), windows-amd64. Binaries build with **default features** (matches `cargo install`). Report which specific (bin, platform) cell failed, not just "a build leg." |
| Release event didn't trigger Homebrew | Releases are authored by `github-actions[bot]`; bot events don't trigger workflows | The `workflow_run` trigger handles this; `head_branch` in the workflow_run payload IS the tag name for tag-triggered runs (verified empirically — and the payload has no `ref` field, whatever a reviewer may claim). |
| Image verify fails on the tag run for a specific bin | Central signer/verify regression | Check the central repo pin in `pipeline.yml` (both `docker-sign`/`docker-verify` matrix cells reference the same SHA) and `references/platform-constraints.md` of the modeled-information-format skill before anything else. |

## Phase 5 — Independent workstation verification

In-pipeline success is necessary; this is the acceptance test. Run from
the local machine, in a scratch dir:

```bash
gh release download v<X.Y.Z> --repo <owner>/<repo>
# Expect 13 assets: 10 binaries (mif-cli + mif-mcp x 5 platforms), one
# combined mif-rs-<X.Y.Z>-sbom.cdx.json, one mif-rs-<X.Y.Z>-source.tar.gz,
# one mif-rs-<X.Y.Z>-checksums.txt

for BIN in mif-cli mif-mcp; do
  for PLATFORM in linux-amd64 linux-arm64 macos-arm64 macos-amd64 windows-amd64.exe; do
    f="${BIN}-<X.Y.Z>-${PLATFORM}"
    gh attestation verify "$f" --repo <owner>/<repo>                  # provenance
    gh attestation verify "$f" --repo <owner>/<repo> \
      --predicate-type https://cyclonedx.org/bom                      # SBOM binding
  done
done
shasum -a 256 -c mif-rs-<X.Y.Z>-checksums.txt

# crates.io: needs a User-Agent or the API/CDN rejects silently — check
# every publishable crate armed for this release (resolved dynamically, not
# hardcoded, so a newly added crate is covered without editing this skill)
for NAME in $(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.publish != []) | .name'); do
  curl -fsSL -A 'release-check' \
    -O "https://static.crates.io/crates/${NAME}/${NAME}-<X.Y.Z>.crate"
  gh attestation verify "${NAME}-<X.Y.Z>.crate" --repo <owner>/<repo>
  echo "crates.io max_version for ${NAME}:"
  curl -s -A 'release-check' "https://crates.io/api/v1/crates/${NAME}" \
    | jq -r .crate.max_version
done

# Container images (digest per bin from the pipeline run's docker job outputs —
# release-docker.yml's image-digests output is a JSON object keyed by bin name):
for BIN in mif-cli mif-mcp; do
  gh attestation verify "oci://ghcr.io/<owner>/<repo>/${BIN}@<digest-for-this-bin>" \
    --repo <owner>/<repo> \
    --signer-workflow <owner>/.github/.github/workflows/sign-and-attest.yml \
    --predicate-type https://slsa.dev/provenance/v1
done
```

Check **exit codes**, not grepped output — a filtered pipe that matches
nothing looks identical to success (this mistake was made once; silence
is not success). No `--signer-workflow` flag for binaries and crates:
those are attested by this repo's own workflows. The container images DO
need `--signer-workflow`: they are signed by the central workflow.

## Final report

Summarize for the user: version, merge commit, tag; per-channel status
(GitHub Release / crates.io **per crate**, every publishable crate / container images
**per bin**, both / Homebrew **per bin**, both); workstation verification
results; and anything from the failure playbook that fired. If any
channel is incomplete, say exactly what is pending, which specific
crate/bin it affects, and what unblocks it.
