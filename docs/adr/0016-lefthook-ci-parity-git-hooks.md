---
title: "Lefthook Git Hooks: Fast Pre-Commit, Full CI-Parity Pre-Push"
description: "Adopt Lefthook-managed git hooks — a fast fmt-only pre-commit check and a full CI-parity pre-push check (fmt, clippy, test, doc, cargo deny) — so a push that would fail CI is caught locally before it consumes CI minutes."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: process
tags:
  - adr
  - git-hooks
  - ci
  - developer-experience
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0009-pedantic-clippy-lint-groups.md
---

# ADR-0016: Lefthook Git Hooks: Fast Pre-Commit, Full CI-Parity Pre-Push

## Status

Accepted

## Context

### Background and Problem Statement

Before this decision, nothing locally prevented a contributor from pushing
code that would fail CI (`cargo clippy -D warnings`, `cargo test`, `cargo deny
check`). The only safeguard was `CLAUDE.md`'s documented "full CI check" — a
command sequence contributors were expected to run manually before pushing —
relying entirely on manual discipline with no automated local enforcement.

### Current Limitations

1. **No automated local enforcement**: a contributor (or an agent) can commit
   and push code that fails clippy, the test suite, or `cargo deny check`,
   and the first signal is a red CI run minutes later.
2. **Manual discipline does not scale**: `CLAUDE.md`'s "run this before
   pushing" convention depends on every contributor remembering to run it,
   every time, with no tooling backstop if they forget.
3. **A "CI-parity" check that isn't actually CI-parity is worse than none**:
   this repository's own history demonstrates the failure mode directly. The
   first implementation of these hooks (commit `407db4d`) added a pre-push
   stage running `cargo test` and `cargo doc`, but omitted the
   `RUSTFLAGS`/`RUSTDOCFLAGS="-D warnings"` environment variables that CI sets
   at the workflow level. An independent review caught this gap five minutes
   later. Without those variables, a plain `rustc`/`rustdoc` warning outside
   clippy's own lint set — an unused variable warning during doctest
   compilation, for example, which clippy does not itself lint — could pass
   the local pre-push check yet still fail CI, defeating the entire purpose
   of calling the hook "CI-parity." The gap was closed the same day, in the
   immediate follow-up commit `1de8c58`.

## Decision Drivers

### Primary Decision Drivers

1. **Commit must stay cheap**: committing is the frequent, low-stakes action
   in the inner loop; a local hook must not slow it down with checks whose
   value is caught later anyway by CI.
2. **Push must be genuinely CI-equivalent, not merely similar**: pushing is
   the less-frequent, higher-stakes action — the point at which code leaves
   the machine — so a "pass locally" result at push time must mean what it
   claims to mean, including matching CI's exact environment variables, or it
   is actively misleading.

### Secondary Decision Drivers

1. **No new infrastructure**: use a hook manager that installs with a single
   command and requires no additional CI-side changes.

## Considered Options

### Option 1: No local hooks; rely on CLAUDE.md and contributor discipline (status quo)

**Description**: Continue relying solely on `CLAUDE.md`'s documented manual
"full CI check" command and contributor discipline, with no local git hook
enforcement.

**Advantages**:

- Zero setup; nothing new to install or maintain.

**Disadvantages**:

- Nothing stops a push that will fail clippy, `cargo test`, or `cargo deny
  check` until CI reports it minutes later — a slower and more expensive
  feedback loop than catching it before the push ever leaves the machine.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: None.
- **Ecosystem Risk**: High. Every forgotten manual check is a CI-minute cost
  and a red commit visible on a pull request.

### Option 2: Lefthook-managed pre-commit (fast) and pre-push (full CI parity) hooks (chosen)

**Description**: Adopt [Lefthook](https://github.com/evilmartians/lefthook)
to manage two hook stages, installed via `lefthook install`:

- **pre-commit**: `cargo fmt --all -- --check` only — catches the single most
  common CI-fail class cheaply, without slowing down frequent commits.
- **pre-push**: full CI parity — `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --all-features --verbose` (with
  `RUSTFLAGS="-D warnings"`), `cargo doc --workspace --no-deps --all-features`
  (with `RUSTDOCFLAGS="-D warnings"`), and `cargo deny check` — each command
  and environment variable matching what CI actually runs, not an
  approximation of it.

**Advantages**:

- The commit/push split keeps the frequent action (commit) cheap while
  making the less-frequent, higher-stakes action (push) fully CI-equivalent.
- `RUSTFLAGS`/`RUSTDOCFLAGS="-D warnings"` at the pre-push stage match CI's
  own workflow-level environment exactly, closing the specific gap a plain
  clippy-only check would leave open.
- Bypassable in a genuine emergency (`--no-verify`), without requiring any
  CI-side change.

**Disadvantages**:

- Provides no protection at all to a contributor who bypasses hooks
  explicitly or who has never run `lefthook install`.

**Risk Assessment**:

- **Technical Risk**: Low. Lefthook is a thin wrapper around commands this
  repository already runs in CI.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

### Option 3: Run the full CI-parity suite on every commit, not just on push

**Description**: Run the same fmt/clippy/test/doc/deny suite at the
pre-commit stage as at pre-push, rather than reserving the full suite for
push.

**Advantages**:

- The earliest possible feedback: a contributor never even has a local commit
  on record that fails clippy, `cargo test`, or `cargo deny check`.

**Disadvantages**: Needlessly slows down the frequent commit-early,
commit-often inner loop for a set of checks whose cost/benefit tradeoff only
really pays off right before code actually leaves the machine (at push time),
not on every single local commit.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: Medium. A multi-minute pre-commit hook discourages
  frequent committing.
- **Ecosystem Risk**: Medium. Contributors are more likely to reach for
  `--no-verify` habitually if commit itself becomes slow.

## Decision

We adopt **Lefthook-managed git hooks**: a fast, fmt-only pre-commit hook and
a full CI-parity pre-push hook, installed via `lefthook install`.

The first implementation of this hook set (commit `407db4d`) had a real gap:
the pre-push stage's `test` and `doc` commands were missing
`RUSTFLAGS`/`RUSTDOCFLAGS="-D warnings"`, the same environment variables CI
sets at the workflow level. An independent review caught this within five
minutes of the hooks being added, and it was closed the same day in commit
`1de8c58`. This is worth stating plainly as part of how the decision was
actually validated, not glossed over — a "CI-parity" hook that silently
diverges from CI's environment is not CI-parity, and this repository shipped
that exact gap once before catching it.

Gitleaks secrets-scanning was deliberately **not** added as a hook in this
pass, despite `.gitleaks.toml` already existing in the repository — the
gitleaks binary itself is not installed on the author's local machine, and
auto-provisioning a hook that depends on a tool that isn't actually usable
yet was judged worse than simply omitting it for now.

## Consequences

### Positive

1. **Failures caught before they leave the machine**: a push that would fail
   clippy, `cargo test`, or `cargo deny check` is now caught locally, before
   it consumes CI minutes or produces a red commit visible on a pull request.
2. **Commit stays cheap, push is fully CI-equivalent**: the pre-commit/pre-push
   split keeps the frequent action (commit) cheap while making the
   less-frequent, higher-stakes action (push) fully CI-equivalent, not merely
   similar to CI.

### Negative

1. **No protection without opt-in**: a contributor who bypasses hooks
   explicitly (`--no-verify`) or who has never run `lefthook install` gets no
   protection at all from these hooks. This is a local convenience layer that
   saves round-trips to CI for contributors who have it installed and don't
   bypass it; it is not, and cannot be, a substitute for CI itself as the
   actual gate.

### Neutral

1. Gitleaks secrets-scanning was deliberately not added as a hook in this
   pass, despite `.gitleaks.toml` already existing in the repository, because
   the gitleaks binary is not installed on the author's local machine.

## Decision Outcome

The decision achieves its primary objective — a pre-push hook that is
genuinely CI-equivalent, not merely similar — measured by: `lefthook.yml`'s
`pre-push` commands set `RUSTFLAGS: "-D warnings"` on the `test` command and
`RUSTDOCFLAGS: "-D warnings"` on the `doc` command, matching
`ci-checks.yml`'s own workflow-level `RUSTFLAGS: "-D warnings"` and job-level
`RUSTDOCFLAGS: "-D warnings"` exactly.

## Related Decisions

- [ADR-0009: Pedantic Clippy Lint Groups](https://modeled-information-format.github.io/mif-rs/adr/0009-pedantic-clippy-lint-groups/) — the lint policy this pre-push hook enforces at `cargo clippy -D warnings` time.

## Links

- [Lefthook](https://github.com/evilmartians/lefthook) — the git hooks manager this decision adopts.
- [Lefthook: `env`](https://lefthook.dev/configuration/env/) — configuration reference for setting per-command environment variables (the mechanism `lefthook.yml`'s pre-push stage uses for `RUSTFLAGS`/`RUSTDOCFLAGS`).
- [Evil Martians, "Lefthook: knock your team's code back into shape"](https://evilmartians.com/chronicles/lefthook-knock-your-teams-code-back-into-shape) — the split-hook rationale (fast pre-commit, thorough pre-push) this ADR mirrors.
- [Cargo Book: Environment Variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html) — documents `RUSTFLAGS`/`RUSTDOCFLAGS`, the variables whose omission caused the CI-parity gap this ADR describes.

## More Information

- **Date**: 2026-07-03
- **Source**: commits `407db4d` ("chore(hooks): add Lefthook git hooks mirroring this repo's own CI") and `1de8c58` ("fix(hooks): close CI-parity gaps found by independent review")

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| pre-push `test` command sets `RUSTFLAGS: "-D warnings"`; pre-push `doc` command sets `RUSTDOCFLAGS: "-D warnings"`, matching `ci-checks.yml`'s workflow-level and job-level settings | lefthook.yml | 28-46 | accepted |

**Summary:** Verified against the current `lefthook.yml` that the gap found on
the day these hooks were introduced (missing `RUSTFLAGS`/`RUSTDOCFLAGS` at the
pre-push stage) was in fact closed, and remains closed.

**Action Required:** None — this ADR documents current, already-adopted
practice.
