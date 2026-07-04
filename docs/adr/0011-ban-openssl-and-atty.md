---
title: "Ban openssl and atty; Use rustls and std::io::IsTerminal"
description: "Deny the openssl and atty crates workspace-wide via cargo-deny's [bans] section, enforced automatically in CI, favoring rustls for TLS and the standard library's std::io::IsTerminal for TTY detection."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: supply-chain
tags:
  - adr
  - supply-chain
  - dependencies
  - security
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
---

# ADR-0011: Ban openssl and atty; Use rustls and std::io::IsTerminal

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs`'s `deny.toml` denies the `openssl` and `atty` crates workspace-wide,
enforced automatically by `cargo deny check` in CI — not a written policy that
could silently lapse, but a real, automated gate that fails the build if
either crate enters the dependency graph. This ADR documents that decision
and its rationale.

### Current Limitations

1. **`openssl` couples the build to a system C library**: `openssl` links a
   system OpenSSL installation with its own build requirements and CVE
   history, a real and recurring source of platform-specific build failures
   across the workspace's six target platforms (`x86_64`/`aarch64` Linux gnu,
   `x86_64`/`aarch64` Apple, `x86_64` Windows MSVC).
2. **`atty` is unmaintained with no remaining reason to exist**: `atty`
   (TTY detection) has been unmaintained since Rust 1.70 stabilized
   `std::io::IsTerminal`, which covers the identical need from the standard
   library.

## Decision Drivers

### Primary Decision Drivers

1. **Reduce attack surface and build-system complexity**: a system C library
   with its own build requirements and CVE history is a real source of
   platform-specific build failures; a pure-Rust or standard-library
   alternative avoids both.
2. **No unmaintained transitive dependencies where a maintained alternative
   exists**: an unmaintained crate should not remain in the dependency graph
   once the standard library or an actively maintained crate covers the
   identical functionality.

### Secondary Decision Drivers

1. **Deterministic, portable builds**: the workspace targets six platforms
   (`x86_64`/`aarch64` Linux gnu, `x86_64`/`aarch64` Apple, `x86_64` Windows
   MSVC); a dependency that requires a system-installed library at build time
   works against that portability goal.
2. **Automated enforcement over documentation**: `cargo-deny`'s `[bans]`
   section turns this policy into a CI gate rather than a convention
   contributors must remember and apply by hand.

## Considered Options

### Option 1: Allow `openssl` for any future TLS need

**Description**: Leave `openssl` unbanned, in case some future crate this
workspace adds needs TLS and only offers an OpenSSL-backed implementation.

**Advantages**: Keeps every crate on crates.io eligible for use regardless of
which TLS backend it defaults to, with no upfront constraint on future
dependency selection.

**Disadvantages**: Couples the build to a system OpenSSL installation and
version — a real and recurring source of platform-specific build failures —
plus `openssl`'s own historical CVE surface, since it wraps a system C
library rather than memory-safe Rust.

**Risk Assessment**:

- **Technical Risk**: High. System OpenSSL version/build mismatches are a
  well-documented, recurring class of build failure across platforms.
- **Schedule Risk**: Medium. Platform-specific build failures surface late,
  typically in CI or on a contributor's machine, not at dependency-add time.
- **Ecosystem Risk**: High. A C library dependency with its own CVE history,
  outside Rust's memory-safety guarantees.

### Option 2: Ban `openssl`; mandate `rustls`. Ban `atty`; mandate `std::io::IsTerminal` (chosen)

**Description**: Deny `openssl` outright in `deny.toml`, mandating `rustls`
(a pure-Rust TLS implementation with a smaller attack surface) for any future
TLS need. Separately, deny `atty` (unmaintained since Rust 1.70 stabilized
`std::io::IsTerminal` covering the identical need) in favor of the standard
library.

**Advantages**:

- No system OpenSSL build dependency anywhere in the dependency tree, for any
  crate in the workspace, present or future.
- One fewer unmaintained transitive dependency (`atty`) in the graph.
- Enforced automatically by `cargo deny check` in CI, so the policy cannot
  silently lapse the way an undocumented convention could.

**Disadvantages**:

- Any future crate this workspace adds that only offers an OpenSSL-backed TLS
  implementation (no `rustls`-backed alternative) must be swapped for a
  `rustls`-backed equivalent or excluded from consideration entirely.

**Risk Assessment**:

- **Technical Risk**: Low. `rustls` and `std::io::IsTerminal` are both
  established, actively maintained (or standard-library) alternatives.
- **Schedule Risk**: Low. The ban is already in place and enforced; no new
  crate in the workspace currently depends on either banned crate.
- **Ecosystem Risk**: Low. Removes a system C library dependency and an
  unmaintained transitive crate from the graph.

### Option 3: Leave `atty` un-banned since its own use is narrow and low-risk

**Description**: Argue that `atty`'s use case — TTY detection — is narrow and
low-risk on its face, and leave it un-banned rather than spend a `deny.toml`
entry on it.

**Advantages**: Saves one `deny.toml` entry, and `atty`'s narrow TTY-detection
surface means it carries no known active vulnerability today.

**Disadvantages**: `atty` remains an unmaintained dependency with literally no
reason to keep once `std::io::IsTerminal` covers the identical functionality
natively. Keeping an unmaintained crate around for no benefit is exactly the
kind of dependency-hygiene debt `cargo-deny` exists to prevent.

**Disqualifying Factor**: "low risk" is not the same as "no reason to keep" —
an unmaintained crate with a drop-in standard-library replacement has no
justification for staying in the dependency graph, regardless of how narrow
its own blast radius is.

**Risk Assessment**:

- **Technical Risk**: Low in isolation, but compounds workspace-wide
  dependency-hygiene debt over time.
- **Schedule Risk**: None.
- **Ecosystem Risk**: Medium. Unmaintained crates accumulate silently if no
  gate catches them.

## Decision

We ban **`openssl`** and **`atty`** workspace-wide via `cargo-deny`'s
`[bans]` section, enforced automatically by `cargo deny check` in CI. The
current `deny.toml` reads:

```toml
[bans]
# Deny multiple versions of the same crate
multiple-versions = "warn"
# Deny wildcard dependencies
wildcards = "deny"
# Highlight the crate with the highest version
highlight = "all"

# Deny specific problematic crates
deny = [
    { name = "openssl", wrappers = [], reason = "Use rustls for TLS instead" },
    { name = "atty", wrappers = [], reason = "Use std::io::IsTerminal instead (available in Rust 1.70+)" },
]

skip = []
skip-tree = []
```

`openssl` is replaced by `rustls` for any TLS need; `atty` is replaced by the
standard library's `std::io::IsTerminal`.

## Consequences

### Positive

1. **No system OpenSSL build dependency**: nowhere in the dependency tree,
   for any crate in the workspace, present or future.
2. **One fewer unmaintained transitive dependency**: `atty` is out of the
   graph entirely.

### Negative

1. **Constrains future dependency choices**: any future crate this workspace
   adds that only offers an OpenSSL-backed TLS implementation (no
   `rustls`-backed alternative) must be swapped for a `rustls`-backed
   equivalent or excluded from consideration entirely.

### Neutral

1. `mif-embed`'s HTTP client (`hf-hub`, used to fetch the embedding model from
   the Hugging Face Hub on first use) must itself resolve to a `rustls`-backed
   TLS stack transitively for this ban to hold across the whole dependency
   graph, not just at the direct-dependency level. `Cargo.lock` confirms this:
   `hf-hub` v0.5.0 depends on `ureq` v3.3.0, whose own dependency list includes
   `rustls`, `rustls-pki-types`, and `webpki-roots` — no `openssl` anywhere in
   the tree. `deny.toml`'s `[licenses.clarify]` and `allow` entries for `ring`
   and `CDLA-Permissive-2.0` (the license covering `webpki-roots`'s bundled
   Mozilla CA root data) corroborate this: both are artifacts of `rustls`'s
   own dependency chain, present because `hf-hub` resolves through `ureq`'s
   `rustls`-backed default, not an OpenSSL-backed one.

## Decision Outcome

The decision achieves its primary objective — no `openssl` or `atty` anywhere
in the workspace's dependency graph — measured by: `cargo deny check bans`
passes with zero violations (confirmed: running it against this workspace
returns `bans ok`), and neither `openssl` nor `atty` appears anywhere in
`Cargo.lock` or `cargo tree` for the workspace (confirmed: `grep -n
"openssl" Cargo.lock` and a search for `name = "atty"` both return no
matches).

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace, Not a Root Package](https://modeled-information-format.github.io/mif-rs/adr/0003-virtual-cargo-workspace/) — establishes the workspace structure this ban applies across.

## Links

- [cargo-deny documentation](https://embarkstudios.github.io/cargo-deny/) — the `[bans]` section this ADR's policy is enforced through.
- [RustSec Advisory Database](https://rustsec.org) — tracks unmaintained and vulnerable crates, including `atty`'s unmaintained status.
- [`rustls`](https://github.com/rustls/rustls) — the pure-Rust TLS implementation mandated in place of `openssl`.
- [`std::io::IsTerminal`](https://doc.rust-lang.org/std/io/trait.IsTerminal.html) — the standard-library API mandated in place of `atty`.

## More Information

- **Date**: 2026-07-03
- **Source**: `deny.toml`'s `[bans]` section (retroactively documents an
  established, ongoing policy).

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| `deny = [{ name = "openssl", wrappers = [], reason = "Use rustls for TLS instead" }, { name = "atty", wrappers = [], reason = "Use std::io::IsTerminal instead (available in Rust 1.70+)" }]` | deny.toml | 75-78 | accepted |

**Summary:** `cargo deny check bans` passes with zero violations against the
current workspace; neither `openssl` nor `atty` appears in `Cargo.lock`.

**Action Required:** None — this ADR documents current, already-enforced
policy.
