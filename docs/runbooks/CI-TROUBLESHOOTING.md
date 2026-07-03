---
id: how-to-troubleshoot-ci-failures
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/ci
title: How to Troubleshoot a Failing mif-rs CI Run
tags:
  - how-to
  - ci
  - troubleshooting
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
relationships:
  - type: relates-to
    target: docs/runbooks/RELEASING.md
  - type: relates-to
    target: docs/runbooks/DEPENDENCY-UPDATES.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Troubleshoot a Failing mif-rs CI Run
  entity_type: how-to-guide
---

# How to Troubleshoot a Failing mif-rs CI Run

Diagnose and fix a failing workflow run on a pull request or push to `main`
in the `mif-rs` workspace, and reproduce the failure locally before pushing a
fix.

## Prerequisites

- `gh` CLI authenticated against `modeled-information-format/mif-rs`.
- A local clone with the same Rust toolchain CI uses (`stable`, plus `1.92`
  for MSRV checks: `rustup install 1.92`).
- `just` installed (the local task runner; run `just` with no arguments to
  list every recipe).

## Step 1 — Read the failing run

```bash
gh run list --status failure --limit 10
gh run view <run-id>
gh run view <run-id> --log-failed
gh run view <run-id> --log > ci-logs.txt   # full logs, if you need more context
```

Or in the browser: **Actions** → the failed run → the failed job (red X) →
expand the failed step → **"Search logs"** to jump to the error.

To retry without changing anything (useful for a suspected flake):

```bash
gh run rerun <run-id> --failed
gh run rerun <run-id>            # re-run the entire workflow
```

## Step 2 — Reproduce locally

`mif-rs` is orchestrated top-level by `pipeline.yml`, which calls
`ci-checks.yml` (fmt, clippy, test, doc, deny, msrv), `ci-coverage.yml`
(coverage), and the org's `pin-check` / `validate-workflows` (actionlint)
reusable workflows. Reproduce the core gate locally in one command:

```bash
just check    # = fmt-check + lint + test + doc-build + deny (matches CI, minus msrv)
just msrv     # separate recipe: cargo +1.92 check --all-features
```

Raw `cargo` equivalents, if you need to run one check in isolation:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --verbose
cargo doc --workspace --no-deps --all-features
cargo deny check
cargo +1.92 check --all-features
```

## Clippy failures

**Workflow:** `pipeline.yml` → `ci-checks.yml` (`clippy` job)
**Command:** `cargo clippy --all-targets --all-features -- -D warnings`

The workspace runs clippy's `pedantic` + `nursery` + `cargo` lint groups
(`[workspace.lints]` in the root `Cargo.toml`), so failures are often stricter
than clippy's defaults. A subset is denied as hard errors regardless of `-D
warnings`, because they're incompatible with library code in this workspace:

| Lint | Cause | Fix |
|---|---|---|
| `clippy::unwrap_used` | Called `.unwrap()` outside `#[cfg(test)]` | Use `?` or an explicit match |
| `clippy::expect_used` | Called `.expect()` outside `#[cfg(test)]` | Use `?` or an explicit match |
| `clippy::panic` | Used `panic!()` in library code | Return `Result` instead |
| `clippy::todo` / `clippy::unimplemented` | Placeholder code | Implement it or remove it |
| `clippy::dbg_macro` | Left `dbg!()` in code | Remove it |
| `clippy::print_stdout` / `clippy::print_stderr` | Used `println!`/`eprintln!` in library code | Use logging; `mif-cli`/`mif-mcp` exempt themselves with `#![allow(...)]` at the crate root since a CLI/server needs to print |
| `clippy::missing_errors_doc` / `missing_panics_doc` | *(allowed workspace-wide — opt-in only)* | N/A |

`#[cfg(test)]` code is exempt from `unwrap_used`/`expect_used`/`dbg_macro`/
`print_stdout` via `clippy.toml`'s `allow-*-in-tests` settings — use plain
`.unwrap()` in tests, not `.unwrap_or_default()` workarounds.

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings   # see all warnings
cargo clippy --workspace --fix --all-targets --all-features            # auto-fix what it can
```

Only suppress a lint with a genuine reason, inline, with a comment — never a
blanket allow in a crate root or `Cargo.toml`:

```rust
#[allow(clippy::too_many_arguments)]  // Builder pattern requires all fields
fn create_widget(a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32) -> Widget {
    // ...
}
```

## Format failures

**Workflow:** `pipeline.yml` → `ci-checks.yml` (`fmt` job)
**Command:** `cargo fmt --all -- --check`

```bash
cargo fmt --all                 # fix
cargo fmt --all -- --check      # check without modifying
```

Configured in `rustfmt.toml` (workspace root): `max_width = 100`, `edition =
"2024"`, Unix newlines, `reorder_imports`/`reorder_modules` both on. If
`cargo fmt` produces unexpected results, confirm your local toolchain matches
CI's `stable` channel — the unstable options at the bottom of `rustfmt.toml`
are commented out and only apply under `cargo +nightly fmt`.

## MSRV failures

**Workflow:** `pipeline.yml` → `ci-checks.yml` (`msrv` job)
**Command:** `cargo check --all-features` on toolchain `1.92`
**MSRV:** Rust 1.92 (`rust-version.workspace = true` in every crate)

```bash
rustup install 1.92
cargo +1.92 check --all-features    # matches `just msrv`
```

| Cause | Fix |
|---|---|
| Used a language feature newer than 1.92 | Rewrite with MSRV-compatible syntax, or check which version stabilized it |
| A dependency bumped its own MSRV | Pin it to the last compatible version in the relevant crate's `Cargo.toml` |
| Used a std API newer than 1.92 | Use a polyfill or feature-gate it |

If the MSRV genuinely needs to move: update `rust-version` in
`[workspace.package]` (`Cargo.toml`), update the `msrv` input default in
`ci-checks.yml`, and call it out in the changelog as a potentially breaking
change.

## cargo-deny failures

**Workflow:** `pipeline.yml` → `ci-checks.yml` (`deny` job)
**Command:** `cargo deny check`
**Configuration:** `deny.toml` (workspace root)

```bash
cargo deny check advisories   # RustSec DB — all advisory types denied
cargo deny check licenses     # SPDX allow-list only
cargo deny check bans         # openssl, atty denied
cargo deny check sources      # crates.io only
```

**Advisory failure** (`error[vulnerability]: RUSTSEC-2024-XXXX`): update the
crate (`cargo update -p <crate-name>`), or — if no fix exists — add a dated,
commented exception to `deny.toml`'s `[advisories] ignore`.

**License failure** (`error[rejected]: license 'X' is not in the allow
list`): either drop the dependency, or add the SPDX identifier to
`[licenses] allow` if it's genuinely acceptable — see
[DEPENDENCY-UPDATES.md](DEPENDENCY-UPDATES.md) for the current allow-list.

**Ban failure** (`error[banned]: crate 'openssl' is banned`): find what pulls
it in (`cargo tree -i openssl`), then either switch to an alternative that
doesn't depend on it, or enable a `rustls` feature flag on the offending
crate instead.

**Source failure** (`error[unknown-registry]: ...`): only crates.io is
allowed. A needed git dependency must be added to `deny.toml`'s
`[sources] allow-git`.

## Test failures

**Workflow:** `pipeline.yml` → `ci-checks.yml` (`test` job, matrix:
`ubuntu-latest`, `macos-latest`, `windows-latest`)
**Command:** `cargo test --all-features --verbose`

```bash
cargo test --workspace --all-features -- --nocapture     # show output
cargo test -p <crate> test_name                          # one test
cargo test --workspace -- --test-threads=1 pattern       # serialize + filter
cargo test --doc                                         # doc tests only
```

If a test passes on one OS but fails on another, check for: hardcoded `/` or
`\` path separators (use `std::path::PathBuf`), `\n` vs `\r\n` assumptions,
temp-directory path format differences (`std::env::temp_dir()`), or Unix
permission checks that need feature-gating on Windows.

For a flaky test, run it repeatedly before assuming it's a real regression:

```bash
for i in $(seq 100); do cargo test test_name || break; done
```

## Documentation failures

**Workflow:** `pipeline.yml` → `ci-checks.yml` (`doc` job)
**Command:** `cargo doc --no-deps --all-features`
**Environment:** `RUSTDOCFLAGS="-D warnings"`

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo doc --open       # view locally
cargo test --doc       # doc examples compile as doctests
```

Every public item requires a doc comment (`missing_docs = "warn"`
workspace-wide). Common failures: an unresolved intra-doc link (fix the
`[`Type`]` reference), a doc referencing a private type (make it `pub` or
remove the reference), or a doc example that doesn't compile.

## Coverage

**Workflow:** `pipeline.yml` → `ci-coverage.yml` (`coverage` job)
**Tool:** `cargo-llvm-cov` 0.6.14, workspace-wide, all features
**Threshold:** the job's own "Check coverage threshold" step fails (`exit 1`)
if line coverage drops below **90%** — the Codecov upload step's
`fail_ci_if_error: false` only means the job won't fail if the external
Codecov service itself is unreachable, not that the threshold check is
advisory. `coverage` is a separate workflow from `ci-checks.yml` and isn't
part of that workflow's `all-checks-pass` aggregate.

```bash
cargo install cargo-llvm-cov
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
cargo llvm-cov --workspace --all-features --html --output-dir coverage-html
open coverage-html/index.html
cargo llvm-cov --workspace --all-features --summary-only
```

## CodeQL (SAST) failures

**Workflow:** `quality-gates.yml` (`sast` job, calls the org's
`reusable-sast-codeql.yml`)
**Trigger:** push to `main`, every PR, weekly (Monday 06:00 UTC)
**Language:** `rust`, `build-mode: none`

Review findings under **Security → Code scanning alerts**. If a finding is a
genuine false positive, dismiss it with a reason and a comment explaining
why — don't silently ignore it.

| Finding class | Fix |
|---|---|
| Uncontrolled format string | Sanitize input or use a `{}` placeholder, not interpolation |
| Path traversal | Validate and canonicalize user-supplied paths |
| Command injection | Avoid a shell; build the command with `Command::new()` and explicit args |
| Integer overflow | Use `.checked_add()` / `.wrapping_add()` instead of unchecked arithmetic |

## pin-check and actionlint failures

**Workflow:** `pipeline.yml` (`pin-check` and `validate-workflows` jobs, both
calling org reusables). Required check contexts: `pin-check / pin-check` and
`validate-workflows / actionlint`.

- **pin-check** fails when any `uses:` in `.github/workflows/*.yml` isn't
  pinned to a full 40-character commit SHA (a version tag or branch ref
  fails). Resolve the SHA and pin it — see
  [DEPENDENCY-UPDATES.md](DEPENDENCY-UPDATES.md) for the general "update a
  pinned action" flow.
- **validate-workflows** (`actionlint`) fails on workflow YAML syntax or
  semantic errors — a typo'd `needs:` reference, an invalid `if:` expression,
  etc. Run `actionlint` locally against the changed file to see the same
  error CI reports.

## Docker build failures

**Workflow:** `pipeline.yml` (`docker` job, needs `ci` + `gate`, calls
`release-docker.yml`). Runs on every push to `main`/`master` and on tags;
on a PR it builds without pushing (`push: ${{ github.event_name !=
'pull_request' }}`). The job itself is gated on `needs.gate.outputs.has-bin-target
== 'true'` (dynamically resolved from `cargo metadata`, not hardcoded) —
true from day one here, since `mif-cli` and `mif-mcp` both carry `[[bin]]`
targets — so the docker job always runs; it is not tied to any crate's
publish status (all 9 crates are already published — see
[RELEASING.md](RELEASING.md)).

| Error | Cause | Fix |
|---|---|---|
| `failed to solve: Dockerfile: not found` | Missing `Dockerfile` | Confirm it still exists at the repo root |
| `COPY failed: file not found` | Wrong path in `Dockerfile` | Check paths match the `crates/<name>/` layout |
| `denied: denied` on push | Missing registry permissions | Confirm the job has `packages: write` (only granted when `push: true`) |
| `unknown: BIN` / build fails immediately | Missing `--build-arg BIN=<name>` | The `Dockerfile` requires `BIN` (`mif-cli` or `mif-mcp`) to select which workspace binary to build — no default |

The `Dockerfile` uses `cargo-chef` for dependency-layer caching (a `chef`/
`planner`/`builder` multi-stage split): the dependency-only layer is cached
independently of application source changes, so a source-only change
rebuilds in seconds rather than recompiling the whole dependency tree.
`release-docker.yml` builds `linux/amd64` only by default — `linux/arm64`
(QEMU-emulated, much slower even with caching) is added only for tag pushes
(`pipeline.yml`'s `docker` job sets `platforms` conditionally on
`github.ref_type == 'tag'`).

Test locally:

```bash
docker build --build-arg BIN=mif-cli -t mif-cli:test .
docker run --rm mif-cli:test --version

# Both platforms, matching what CI builds only for an actual tag push:
docker buildx build --platform linux/amd64,linux/arm64 \
  --build-arg BIN=mif-cli -t mif-cli:test .
```

## Release workflow failures

**Workflow:** `release.yml`, **Trigger:** push of a `v*.*.*` tag.

The release matrix builds both `mif-cli` and `mif-mcp` across 5 platforms and
publishes all 9 crates (`mif-core`, `mif-problem`, `mif-schema`,
`mif-frontmatter`, `mif-ontology`, `mif-embed`, `mif-store`, `mif-cli`,
`mif-mcp`) independently to crates.io. See
[RELEASING.md](RELEASING.md) for the full chain and monitoring steps; the
common failure points are:

| Error | Cause | Fix |
|---|---|---|
| `Cargo.toml version mismatch` | Workspace `version` doesn't match the tag | Ensure `version = "X.Y.Z"` in `[workspace.package]` matches tag `vX.Y.Z` |
| macos-amd64 build fails | Cross-target leg (`x86_64-apple-darwin` on `macos-latest`) | Check the toolchain step's `targets:` input; the other 4 legs build natively |
| Cargo Audit gate fails | Real advisory in `Cargo.lock` | Fix via a normal PR first (see [DEPENDENCY-UPDATES.md](DEPENDENCY-UPDATES.md)), then restart the release |
| Verify Attestations fails | Missing/unverifiable attestation | The fail-closed gate worked — no release was created. Fix the cause and release with a **new** tag; never re-run against an existing one |
| crates.io publish fails, "No Trusted Publishing config found" | Trusted Publishing not configured for that crate | One-time setup per crate on crates.io: **Settings → Trusted Publishing**, workflow `publish.yml`, environment `release` |
| Homebrew formula not updated | `workflow_run` trigger missed, or tap token missing | `gh workflow run package-homebrew.yml -f version=X.Y.Z -f dry_run=false`; check `HOMEBREW_TAP_TOKEN` |

## Dependabot merge conflicts

When a Dependabot PR conflicts on `Cargo.lock`, ask it to rebase first:

```bash
gh pr comment <number> --body "@dependabot rebase"
```

If that doesn't clear it:

```bash
gh pr checkout <number>
git merge main
cargo update -p <crate-from-dependabot-pr>   # regenerate the lockfile
git add Cargo.lock
git commit -m "chore: resolve merge conflict in Cargo.lock"
git push
```

Useful comment commands: `@dependabot rebase`, `@dependabot recreate`,
`@dependabot merge`, `@dependabot cancel merge`, `@dependabot ignore this
major version`, `@dependabot ignore this dependency`.

## Quick reference: CI jobs and local equivalents

| CI job | Local equivalent | In `ci-checks.yml`'s `all-checks-pass`? |
|---|---|---|
| Format | `cargo fmt --all -- --check` | Yes |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Yes |
| Test (ubuntu/macos/windows) | `cargo test --workspace --all-features --verbose` | Yes |
| Documentation | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` | Yes |
| Cargo Deny | `cargo deny check` | Yes |
| MSRV | `cargo +1.92 check --all-features` | Yes |
| Coverage | `cargo llvm-cov --workspace --all-features` (90% threshold) | No — separate workflow (`ci-coverage.yml`) |
| CodeQL (SAST) | Static analysis, no local equivalent | No — separate workflow (`quality-gates.yml`) |
| Security Audit | `cargo audit --deny warnings` | No — separate workflow (`security-audit.yml`) |
| pin-check / actionlint | No local equivalent for pin-check; `actionlint` for the linter | No — separate `pipeline.yml` jobs |

Run the full `all-checks-pass` set at once:

```bash
just check && just msrv
```

Every job above now passes locally, or you know exactly which one to fix
before pushing again.
