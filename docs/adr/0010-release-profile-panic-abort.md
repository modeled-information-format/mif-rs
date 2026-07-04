---
title: "Release Profile: panic = \"abort\", strip, and Thin LTO"
description: "Build mif-rs release binaries with panic = \"abort\", strip = true, and lto = \"thin\" for smaller, faster distributed binaries, with a separate release-debug profile that keeps debug symbols for profiling."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: build
tags:
  - adr
  - cargo-profile
  - release
  - build
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0003-virtual-cargo-workspace.md
  - 0009-pedantic-clippy-lint-groups.md
---

# ADR-0010: Release Profile: panic = "abort", strip, and Thin LTO

## Status

Accepted

## Context

### Background and Problem Statement

`mif-cli` and `mif-mcp` are distributed as compiled binaries to end users —
per-platform archives plus a Homebrew tap formula, built and published by the
release pipeline documented in `docs/runbooks/RELEASING.md`. Unlike an
internal-only build that only this workspace's CI ever runs, a release
binary's size and runtime performance are a cost every end user who installs
it actually pays. The root `Cargo.toml`'s `[profile.release]` needed settings
that reflect that distribution reality, not just the workspace's default dev
build.

### Current Limitations

1. **Default `panic = "unwind"` in release carries unwinding-table cost with
   no compensating benefit here**: this workspace's own
   `[workspace.lints.clippy]` table already sets `panic = "deny"`, so library
   code in this workspace cannot ship a `panic!` call in the first place —
   there is no genuine panic-unwind-and-recover code path (for example,
   catching a panic across a plugin/FFI boundary) for a release binary to
   ever exercise.
2. **Default builds carry full debug symbols into distributed binaries**:
   without an explicit `strip` setting, every binary shipped to end users
   would carry debug symbols it has no use for outside a profiling session.
3. **No dedicated profile for profiling a release-shaped binary**: a
   contributor who needs a release-shaped stack trace or profiling session
   has no separate profile to reach for once `strip = true` is set on
   `[profile.release]` itself.

## Decision Drivers

### Primary Decision Drivers

1. **User-facing binary cost**: `mif-cli` and `mif-mcp` release binaries are
   installed and run by end users (directly and via the Homebrew tap), so
   their size and runtime performance are a real, user-facing cost, not
   merely an internal build-time concern.
2. **No genuine panic-unwind use case**: this workspace's `clippy::panic`
   lint is already `deny`'d in `[workspace.lints.clippy]`, so a
   panic-unwind-and-recover pattern was never a real use case in this
   workspace's library code to begin with.

### Secondary Decision Drivers

1. **Distributed-binary download cost**: `mif-cli` and `mif-mcp` ship as
   per-platform archives plus a Homebrew tap formula (`docs/runbooks/RELEASING.md`),
   so shaving debug-symbol weight off every archive reduces what each end
   user downloads and installs.
2. **A release-shaped profiling path must stay reachable**: Current
   Limitation 3 notes that once `strip = true` lands on `[profile.release]`,
   contributors need a separate profile to still get a release-shaped stack
   trace or profiling session — whatever is chosen can't remove that path
   outright.
3. **CI build-time budget across the release pipeline's multi-platform
   matrix**: build time is a real constraint at this project's current scale
   (two small binary crates), which weighs against maximal optimization
   settings like fat LTO.

## Considered Options

### Option 1: Default `panic = "unwind"` in release

**Description**: Leave `[profile.release]` at Cargo's default panic
strategy, keeping unwinding tables in every release binary.

**Advantages**:

- Matches Cargo's out-of-the-box default, so there is no explicit profile
  setting to understand, document, or maintain.
- Preserves standard unwind semantics (e.g., `catch_unwind`) if a genuine
  unwind-and-recover need ever arose in a future dependency.

**Disadvantages**: Larger binaries with no compensating benefit here, since
this workspace's own lint policy already prevents library code from
panicking in the first place (`clippy::panic` is `deny`'d) — there is no
real unwind-and-recover code path a release binary would ever exercise.

**Risk Assessment**:

- **Technical Risk**: Low. Cargo's default; no behavior change from status
  quo.
- **Schedule Risk**: None.
- **Ecosystem Risk**: Medium. Every distributed binary carries unwinding-table
  size with no user-facing benefit, indefinitely.

### Option 2: `panic = "abort"`, `strip = true`, `lto = "thin"`, with a separate `release-debug` profile (chosen)

**Description**: Set `panic = "abort"`, `strip = true`, and `lto = "thin"` on
`[profile.release]`. Add a separate `[profile.release-debug]` that inherits
`[profile.release]`'s optimization settings but sets `debug = true` and
`strip = false`, for when a release-shaped stack trace or profiling session
is actually needed.

**Advantages**:

- Smaller, faster release binaries for end users, with no real cost, since
  the panic-unwind-and-recover path `panic = "abort"` gives up was never a
  genuine use case here.
- `release-debug` keeps a release-shaped profiling path available without
  reverting `[profile.release]` itself.

**Disadvantages**:

- Gives up the panic-unwind-and-recover code path entirely and
  workspace-wide — if a future dependency genuinely needed `catch_unwind`,
  `panic = "abort"` would have to be reverted for the whole release
  profile, not just the crate that needed it.
- `lto = "thin"` and `codegen-units = 1` increase release build time versus
  a no-LTO baseline, though less than fat LTO would (see Option 3).

**Risk Assessment**:

- **Technical Risk**: Low. `panic = "abort"`, `strip`, and `lto = "thin"` are
  all well-established, widely used Cargo profile settings.
- **Schedule Risk**: Low.
- **Ecosystem Risk**: Low.

### Option 3: Full `lto = "fat"` instead of `"thin"`

**Description**: Use `lto = "fat"` (formerly `lto = true`) on
`[profile.release]` for a marginal further size/speed gain over thin LTO.

**Advantages**:

- Marginally smaller, faster binaries than thin LTO, since fat LTO optimizes
  across the entire dependency graph rather than in parallelizable
  partitions.

**Disadvantages**: The additional build-time cost of fat LTO is not worth the
marginal further size/speed gain for this workspace's CI budget, at this
project's current scale (two small binary crates).

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: Medium. Fat LTO meaningfully increases release build
  time across the release pipeline's multi-platform build matrix.
- **Ecosystem Risk**: Low.

## Decision

We set **`panic = "abort"`, `strip = true`, and `lto = "thin"`** on
`[profile.release]`, and add a separate **`[profile.release-debug]`** profile
that inherits `[profile.release]`'s optimization settings but keeps debug
symbols, for profiling.

## Consequences

### Positive

1. **Smaller, faster release binaries**: end users who install `mif-cli` or
   `mif-mcp` (directly or via the Homebrew tap) get smaller downloads and
   faster-running binaries.
2. **No real cost accepted**: a panic-unwind-and-recover path was never a
   genuine use case in this workspace, given `clippy::panic = "deny"` in
   `[workspace.lints.clippy]` — `panic = "abort"` gives up nothing this
   workspace's own code was relying on.

### Negative

1. **`release-debug` builds required for profiling**: `strip = true` release
   builds carry no debug info at all, so a release-shaped stack trace or
   profiling session requires building `--profile release-debug` instead.

### Neutral

1. `[profile.dev]` is unchanged by this decision: `panic = "unwind"`
   (Cargo's default), no `strip`, `debug = 1` (line tables only) for fast
   local iteration. This decision only changes the release-facing profiles,
   not the inner development loop.

## Decision Outcome

The decision achieves its primary objective — smaller, faster release
binaries with a preserved profiling path — measured by: the root
`Cargo.toml`'s `[profile.release]` table sets `panic = "abort"`,
`strip = true`, and `lto = "thin"` exactly, and `[profile.release-debug]`
exists as a separate profile that inherits those optimizations while
omitting `strip`. Verified directly against the file:

```toml
[profile.dev]
# Faster compile times during development
debug = 1
opt-level = 0

[profile.release]
# Optimize for size and speed
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"
strip = true

[profile.release-debug]
inherits = "release"
debug = true
strip = false
```

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace, Not a Root Package](0003-virtual-cargo-workspace.md) —
  establishes the workspace whose root `Cargo.toml` carries these profiles.
- [ADR-0009: Pedantic Clippy Lint Groups](0009-pedantic-clippy-lint-groups.md) —
  establishes the `clippy::panic = "deny"` lint policy this decision relies
  on as the reason a panic-unwind-and-recover path was never a real use case
  here.

## Links

- [Cargo Book: Profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) —
  reference for `panic`, `strip`, `lto`, `codegen-units`, and custom
  profile inheritance (`inherits`).
- [Cargo Book: Profile Settings — `panic`](https://doc.rust-lang.org/cargo/reference/profiles.html#panic) —
  the `"unwind"` vs `"abort"` strategies this decision chooses between.
- [Cargo Book: Profile Settings — LTO](https://doc.rust-lang.org/cargo/reference/profiles.html#lto) —
  thin vs fat link-time optimization trade-offs referenced in Option 3.
- [`docs/runbooks/RELEASING.md`](../runbooks/RELEASING.md) — this
  workspace's own release pipeline, the end-user distribution context these
  profile settings serve.

## More Information

- **Date**: 2026-07-03 (retroactively documents an established, ongoing
  configuration).
- **Source**: the root `Cargo.toml`'s `[profile.*]` tables;
  `docs/runbooks/RELEASING.md` for the end-user distribution context
  (per-platform binary archives and the Homebrew tap).

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| `[profile.release]` sets `panic = "abort"`, `strip = true`, `lto = "thin"`; `[profile.release-debug]` inherits `release` with `debug = true`, `strip = false` | `Cargo.toml` | 78-89 | accepted |

**Summary:** Read the root `Cargo.toml` directly and confirmed the release
and release-debug profiles match this decision exactly.

**Action Required:** None — this ADR documents current, already-adopted
configuration.
