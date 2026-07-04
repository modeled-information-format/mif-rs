---
title: "cargo-chef Multi-Stage Docker Build for Dependency-Layer Caching"
description: "Restructure mif-rs's Dockerfile into a cargo-chef chef/planner/builder multi-stage build so dependency compilation lands in its own cacheable Docker layer, cutting per-binary CI build time from 25-30 minutes to roughly 11 seconds once that layer is cached."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: build
tags:
  - adr
  - docker
  - ci
  - build-performance
status: accepted
created: 2026-07-03
updated: 2026-07-03
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0013-chainguard-glibc-dynamic-container-base.md
---

# ADR-0012: cargo-chef Multi-Stage Docker Build for Dependency-Layer Caching

## Status

Accepted

## Context

### Background and Problem Statement

`mif-rs`'s original Dockerfile used a single stage that copied the whole
workspace and ran `cargo build --release` against it directly. That meant
every CI push recompiled the entire dependency tree from scratch, with no
Docker layer boundary between "install dependencies" and "build the
workspace's own source" — Docker had no way to reuse a cached dependency
build across pushes, because the dependency compile and the source compile
were the same `RUN` layer. Combined with QEMU-emulated `arm64`
cross-compilation running on every push, this measured at 25-30 minutes per
binary, on every single push, regardless of whether any dependency had
actually changed.

### Current Limitations

1. **From-scratch recompilation on every push**: a push that touches only
   this workspace's own source, with zero dependency changes, still paid the
   full cost of recompiling every dependency in the tree.
2. **Unsustainable as the workspace grows**: more crates and more CI runs
   per day multiply a fixed 25-30 minute cost that buys nothing on most
   pushes.
3. **Direct velocity cost**: a slow feedback loop discourages frequent
   pushes and slows PR iteration, on top of the direct GitHub Actions minutes
   cost.

## Decision Drivers

### Primary Decision Drivers

1. **CI build time is a direct cost and a direct velocity cost**: GitHub
   Actions minutes are billed directly, and a slow feedback loop discourages
   frequent pushes and slows PR iteration — both costs compound as the
   workspace grows.
2. **Empirical verification, not reputation**: any proposed fix must be
   verified against a real build, not merely assumed to work from
   cargo-chef's general reputation.

### Secondary Decision Drivers

1. **Minimize added build-system complexity**: any fix should avoid
   introducing new external infrastructure, favoring a solution that adds
   Dockerfile structure only, not new services or tooling outside the build
   itself.
2. **Cache correctness over cache aggressiveness**: the caching mechanism
   must not risk a stale dependency layer surviving a genuine `Cargo.lock`
   change — a faster build that occasionally serves outdated dependencies
   would not be an acceptable trade.

## Considered Options

### Option 1: Accept the 25-30 minute build as the ongoing cost of correctness

**Description**: Keep the single-stage Dockerfile as-is and treat 25-30
minutes per binary, per push, as the fixed cost of a correct build.

**Advantages**:

- Zero added Dockerfile complexity — no new stages, no new build-time
  dependency on cargo-chef, nothing to verify beyond the already-known-good
  single-stage build.

**Disadvantages**:

- Unsustainable as the workspace grows to more crates and more CI runs per
  day.
- Every push that changes zero dependencies still pays the full
  from-scratch compile cost — the expensive part of the build buys nothing
  on most pushes.

**Risk Assessment**:

- **Technical Risk**: None. Nothing changes.
- **Schedule Risk**: High. Every push, and every PR iteration, pays the full
  build time with no way to shorten it.
- **Ecosystem Risk**: Medium. A slow, unchanging feedback loop discourages
  frequent pushes as the workspace and its CI volume grow.

### Option 2: cargo-chef 3-stage Docker build (chosen)

**Description**: A `chef` stage installs cargo-chef; a `planner` stage runs
`cargo chef prepare` to produce a `recipe.json` capturing just the
dependency graph, not the workspace's own source; a `builder` stage runs
`cargo chef cook --release` from that recipe as its own Docker layer, cached
independently of source changes, then performs the real workspace build on
top of that cached layer. Docker's own layer caching means the expensive
dependency-compile layer is reused across pushes whenever `Cargo.lock` is
unchanged.

**Advantages**:

- The expensive dependency-compile step becomes its own Docker layer,
  reused across pushes as long as `Cargo.lock` is unchanged.
- A push that touches only this workspace's own source no longer pays the
  dependency-recompile cost at all.

**Disadvantages**:

- The Dockerfile itself gains real structural complexity: three stages
  (`chef`, `planner`, `builder`) ahead of the final runtime stage, instead of
  one.

**Risk Assessment**:

- **Technical Risk**: Low. cargo-chef is a purpose-built tool for exactly
  this caching pattern; the change is verifiable with a real local build.
- **Schedule Risk**: Low. A three-stage Dockerfile is more to read, but
  no new external infrastructure is required.
- **Ecosystem Risk**: Low.

### Option 3: Drop arm64 from the default CI build path entirely

**Description**: Keep only `amd64` in the default CI build path and accept
slower feedback in exchange for architectural simplicity, dropping `arm64`
from every regular CI run.

**Advantages**:

- Removes QEMU-emulated `arm64` cross-compilation from every regular CI run,
  which was itself a meaningful share of the original 25-30 minute cost.

**Disadvantages**:

- Does not by itself solve the from-scratch dependency-recompile problem
  that cargo-chef addresses directly — a from-scratch `amd64`-only build
  still recompiles every dependency on every push.

**Risk Assessment**:

- **Technical Risk**: Low.
- **Schedule Risk**: Low. Delays the first `arm64` build in a release cycle
  to the tag push itself.
- **Ecosystem Risk**: Low.

## Decision

We adopt **Option 2**, combined with the `arm64`-on-tag-pushes-only slice of
Option 3: this repository's actual current CI/CD workflow builds multi-arch
images only when `github.ref_type == 'tag'`, and `amd64`-only otherwise. The
two are complementary, not substitutes for one another — cargo-chef fixes
the from-scratch dependency-recompile cost, while restricting `arm64` to tag
pushes fixes the QEMU cross-compilation cost, and neither alone would have
addressed the other.

## Consequences

### Positive

1. **Per-binary compile time confirmed at approximately 11 seconds** once
   the dependency layer is cached — down from 25-30 minutes — verified with
   a real local Docker build, not assumed from cargo-chef's general
   reputation.

### Negative

1. **Dockerfile complexity grew**: from a single build stage to three
   (`chef`, `planner`, `builder`) ahead of the final runtime stage.
2. **A genuine `Cargo.lock` change still pays the full dependency-compile
   cost**: that file is exactly the cache key `cargo chef prepare`'s
   `recipe.json` is derived from, so a real dependency change correctly
   invalidates the cached layer.

### Neutral

1. `arm64` images are now built only on tag pushes (releases), not on every
   regular CI run — trading a slightly slower first-`arm64`-build-per-release
   for a faster default inner development loop.

## Decision Outcome

The decision achieves its primary objective — a dependency-compile layer
that is cached across pushes instead of recompiled from scratch — measured
by: a real local `docker build` on this repository's current Dockerfile
completing the per-binary compile step in roughly 11 seconds once the
dependency layer is already cached. That ~11-second figure is carried
forward from the original verified measurement recorded in commit
`44eecee`, not re-measured for this ADR.

## Related Decisions

- [ADR-0013: Chainguard glibc-dynamic Container Base](0013-chainguard-glibc-dynamic-container-base.md) — the runtime stage this Dockerfile's `builder` stage feeds into.

## Links

- [cargo-chef](https://github.com/LukeMathWalker/cargo-chef) — the tool this decision adopts for dependency-layer caching.
- [Docker build cache](https://docs.docker.com/build/cache/) — Docker's own documentation on the layer-caching behavior this decision relies on.
- [Docker multi-stage builds](https://docs.docker.com/build/building/multi-stage/) — the multi-stage pattern (`chef`/`planner`/`builder`) this Dockerfile implements.
- [Fast Rust Docker builds](https://www.lpalmieri.com/posts/fast-rust-docker-builds/) — cargo-chef's author on the caching pattern this ADR adopts.

## More Information

- **Date**: 2026-07-03 (retroactively documents a decision made 2026-07-02).
- **Source**: commit `44eecee` ("fix: use cargo-chef for Docker layer caching,
  amd64-only outside releases"), the current `Dockerfile`, and
  `.github/workflows/pipeline.yml`'s platform-matrix conditional
  (`github.ref_type == 'tag' && 'linux/amd64,linux/arm64' || 'linux/amd64'`).

## Audit

### 2026-07-03

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| chef/planner/builder multi-stage cargo-chef build confirmed as current Dockerfile state | Dockerfile | 5-29 | accepted |

**Summary:** The current `Dockerfile` implements the decision as recorded:
a `chef` stage installs cargo-chef, a `planner` stage runs
`cargo chef prepare`, and a `builder` stage runs `cargo chef cook --release`
against the resulting recipe before building the workspace's own binaries.

**Action Required:** None — this ADR documents current, already-adopted
practice.
