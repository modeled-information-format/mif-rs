---
id: how-to-update-dependencies-mif-rs
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/dependencies
title: How to Manage Cargo and GitHub Actions Dependency Updates
tags:
  - how-to
  - dependencies
  - dependabot
  - cargo-deny
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
relationships:
  - type: relates-to
    target: docs/runbooks/CI-TROUBLESHOOTING.md
  - type: relates-to
    target: docs/runbooks/SECURITY-RESPONSE.md
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Manage Cargo and GitHub Actions Dependency Updates
  entity_type: how-to-guide
---

# How to Manage Cargo and GitHub Actions Dependency Updates

Triage and act on a Dependabot PR, run a manual dependency audit, or add/remove
a dependency in the `mif-rs` workspace while staying inside the license and
supply-chain policy enforced by `deny.toml`.

## Prerequisites

- Write access to `modeled-information-format/mif-rs` (or a fork with a PR open).
- `gh` CLI authenticated.
- `cargo-deny` and `cargo-audit` installed locally for manual checks:
  ```bash
  cargo install cargo-deny cargo-audit
  ```

## Handle an incoming Dependabot PR

Dependabot is configured in `.github/dependabot.yml` across three ecosystems,
all on the same weekly cadence (Mondays 09:00 America/Chicago):

| Ecosystem | Directory | PR limit | Grouped updates (minor + patch) |
|---|---|---|---|
| `cargo` | `/` | 10 | `dev-dependencies` (`proptest*`, `test-*`, `criterion*`), `async-runtime` (`tokio*`, `async-*`), `serde-ecosystem` (`serde*`) |
| `github-actions` | `/` | 5 | all actions grouped together |
| `docker` | `/` | 5 | ungrouped — keeps Dockerfile base-image digest pins fresh |

Every ecosystem uses commit prefix `chore(deps)`, labels `dependencies` (plus
an ecosystem-specific label), and reviewer `modeled-information-format`.

### Step 1 — Check whether it auto-merges

`dependabot-automerge.yml` runs on every Dependabot PR (`opened`,
`synchronize`, `reopened`, gated on `github.actor == 'dependabot[bot]'`) and
calls `gh pr merge --auto --squash` for **patch** and **minor** version
updates only:

```bash
gh pr view <number> --json title,labels,author
```

If the PR is a **major** version bump, auto-merge does not fire — it needs
manual review.

### Step 2 — Review a major bump or a flagged PR

```bash
gh pr diff <number>
```

Read the dependency's changelog for breaking changes, then:

```bash
gh pr review <number> --approve
gh pr merge <number> --squash
```

### Step 3 — Recover from a stuck or conflicted PR

If Dependabot flags a merge conflict in `Cargo.lock`, ask it to rebase first:

```bash
gh pr comment <number> --body "@dependabot rebase"
```

If that doesn't resolve it, check the PR out and regenerate the lockfile
manually:

```bash
gh pr checkout <number>
git merge main
cargo update -p <crate-from-dependabot-pr>
git add Cargo.lock
git commit -m "chore: resolve merge conflict in Cargo.lock"
git push
```

## Run a manual dependency audit

Use this when a Dependabot cycle hasn't caught something yet, or before a
release.

### Step 1 — Update the advisory database and audit

```bash
cargo audit fetch
cargo audit --deny warnings
```

`security-audit.yml` runs the same command daily at 00:00 UTC, on every push
that touches `Cargo.toml`/`Cargo.lock`, and on manual `workflow_dispatch`.

### Step 2 — Run the full supply-chain policy check

```bash
cargo deny check
```

`deny.toml` (workspace root) enforces four categories, each checkable on its
own:

```bash
cargo deny check advisories   # RustSec DB — all advisory types denied
cargo deny check licenses     # SPDX allow-list only
cargo deny check bans         # openssl, atty denied; multiple-versions = warn
cargo deny check sources      # crates.io only
```

The license allow-list (`deny.toml` `[licenses] allow`) is:

```text
MIT, MIT-0, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause,
BSD-3-Clause, ISC, Zlib, MPL-2.0, Unicode-DFS-2016, Unicode-3.0, CC0-1.0,
BSL-1.0, 0BSD
```

`[bans] deny` blocks `openssl` (use `rustls`) and `atty` (use
`std::io::IsTerminal`).

### Step 3 — Inspect the dependency graph

```bash
cargo tree                    # full graph
cargo tree --duplicates       # duplicate transitive versions (warn, not deny)
cargo tree -i <crate-name>    # who pulls in a given crate, and why
```

## Fix a security advisory cargo-audit or Dependabot flags

### Step 1 — Assess it

```bash
cargo audit
cargo tree -i <affected-crate>   # is the vulnerable path actually reachable?
```

### Step 2 — Update the dependency

```bash
cargo update -p <crate-name>
cargo audit
```

### Step 3 — If no fix is available yet

In order of preference:

1. Pin to an unaffected version in the relevant crate's `Cargo.toml`.
2. Add a temporary, dated exception in `deny.toml`:
   ```toml
   [advisories]
   ignore = [
       "RUSTSEC-2024-XXXX",  # No fix available; unexploitable in our usage. Revisit by YYYY-MM-DD.
   ]
   ```
3. Replace the dependency with an alternative crate.
4. Fork and patch it.

### Step 4 — Verify and push

```bash
cargo deny check advisories
cargo test --workspace --all-features
git add Cargo.toml Cargo.lock
git commit -m "fix(deps): address RUSTSEC-XXXX-YYYY in <crate>"
git push
```

## Add a new dependency

### Step 1 — Evaluate it against workspace policy

- Can this be done with `std` or an existing workspace dependency instead?
- Is the license in the `deny.toml` allow-list above?
- Does it avoid pulling in `openssl` or `atty` (check `cargo tree -i`)?
- Does it support the workspace MSRV, Rust 1.92 (`rust-version.workspace = true`
  in every crate's `Cargo.toml`)?
- Is it actively maintained, tested, documented?

### Step 2 — Add it and check its feature footprint

```bash
cargo add <crate-name>            # or --dev for a dev-only dependency
cargo tree -p <crate-name>        # what it pulls in transitively
```

Check default features before accepting them. `jsonschema` in this workspace
is a precedent: its defaults pull in a full `reqwest`/`rustls`/`aws-lc-rs`
HTTP-resolver stack the workspace doesn't use, so it's pinned
`default-features = false` in `[workspace.dependencies]`. If a new
dependency's defaults pull in something unexpected, disable defaults and
enable only the features actually needed.

### Step 3 — Verify

```bash
cargo deny check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

### Step 4 — If the license isn't in the allow-list

1. Confirm it's genuinely a safe permissive license — `cargo deny check
   licenses` names the exact crate and license string.
2. Add it to `deny.toml`:
   ```toml
   [licenses]
   allow = [
       # ...existing licenses...
       "NEW-LICENSE-SPDX",
   ]
   ```
3. State why the license was added in the commit message.

## Remove an unused dependency

```bash
cargo install cargo-machete
cargo machete                                     # find unused dependencies
cargo remove <crate-name>
cargo update
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
```

## Change the Dependabot schedule or grouping

Edit `.github/dependabot.yml`. To add a grouped update:

```yaml
groups:
  new-group-name:
    patterns:
      - "crate-prefix*"
    update-types:
      - "minor"
      - "patch"
```

To ignore a dependency entirely:

```yaml
ignore:
  - dependency-name: "crate-to-ignore"
    versions: [">=2.0.0"]   # or omit to ignore all updates
```

The dependency set is now current with policy, and the audit trail (commit
messages, `deny.toml` exceptions with revisit dates) is in place for anything
that couldn't be fixed immediately.

## Quick Reference

| Task | Command |
|---|---|
| Full supply-chain check | `cargo deny check` |
| Security advisory audit | `cargo audit --deny warnings` |
| Update all dependencies | `cargo update` |
| Update one dependency | `cargo update -p <crate>` |
| Show dependency tree | `cargo tree` |
| Find duplicate versions | `cargo tree --duplicates` |
| Find unused dependencies | `cargo machete` |
| Add a dependency | `cargo add <crate>` |
| Remove a dependency | `cargo remove <crate>` |
| List open Dependabot PRs | `gh pr list --label dependencies` |
