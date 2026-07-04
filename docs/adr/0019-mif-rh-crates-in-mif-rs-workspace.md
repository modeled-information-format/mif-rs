---
title: "Ship the Research-Harness Ontology Engine as mif-rh Crates Inside the mif-rs Workspace"
description: "Package the compiled research-harness ontology engine as three crates (mif-rh, mif-rh-cli, mif-rh-mcp) inside the existing mif-rs workspace, producing two binaries, instead of the standalone harness-ontology-engine repository and unified hoe binary the authorizing RFC proposed."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - packaging
  - workspace
  - mif-rh
  - architecture
status: accepted
created: 2026-07-04
updated: 2026-07-04
author: zircote
project: mif-rs
audience:
  - developers
  - architects
related:
  - 0003-virtual-cargo-workspace.md
  - 0004-libraries-never-depend-on-binaries.md
---

# ADR-0019: Ship the Research-Harness Ontology Engine as mif-rh Crates Inside the mif-rs Workspace

## Status

Accepted

## Context

### Background and Problem Statement

`research-harness-template`'s ADR-0014 ("Compiled ontology engine as a scoped
CLI+MCP proof-of-concept") authorized a compiled reimplementation of its
`resolve-ontology.sh`/`ontology-review.sh` pair. Its accompanying RFC
(`docs/proposals/ontology-engine/rust-rfc-engine-core.md`, lines 76-109)
proposed a concrete shape: a new standalone workspace named
`harness-ontology-engine` with crates `engine-core`/`cli`/`mcp-server`,
producing **one** binary, `hoe`, with `review`/`resolve`/`mcp-serve`
subcommands.

What was actually built is different: three crates (`mif-rh`, `mif-rh-cli`,
`mif-rh-mcp`) inside this pre-existing `mif-rs` workspace, producing **two**
binaries (`mif-rh-cli`, `mif-rh-mcp`) and no unified `hoe`-style binary. The
engine-core/CLI/MCP separation of concerns the RFC required is preserved
exactly — only the naming and packaging changed. No design document recorded
that decision or its rationale; a 2026-07-04 architecture gap analysis
(`mif-rh-punchlist.md`, Risk R-8) flagged the silence. This ADR records the
decision retroactively.

### Current Limitations

1. **The RFC's proposed names still circulate**: readers of the
   `research-harness-template` design-doc set encounter `hoe` and
   `harness-ontology-engine` and find no repository by either name; without a
   recorded decision the mismatch reads as drift rather than choice.
2. **Reconstructed rationale**: like ADR-0015, this record is written after
   the fact. The rationale below is reconstructed from the workspace's own
   established conventions, not transcribed from an original comparison —
   none exists.

## Decision Drivers

### Primary Decision Drivers

1. **The engine is a consumer of this workspace's library crates.** `mif-rh`
   builds directly on `mif-ontology` (extends-chain resolution), `mif-embed`
   (local embeddings), and `mif-problem` (RFC 9457 envelopes). WHEN the
   engine's crates live in the same workspace as those libraries, THE SYSTEM
   SHALL catch a breaking library change in the same PR that introduces it —
   the same driver ADR-0003 records for the workspace itself.
2. **Shared quality and release infrastructure.** A standalone repository
   would need its own clippy/deny/CI/release/attestation stack duplicated
   from this one. Inside `mif-rs`, the mif-rh crates inherit workspace lints,
   `cargo deny` supply-chain policy, the release pipeline's
   `cargo metadata`-driven multi-binary packaging, and signed attestations
   with zero additional configuration.

### Secondary Decision Drivers

1. **ADR-0004's layering already fits**: one library crate plus thin binary
   consumers is this workspace's established shape; the RFC's
   engine-core/cli/mcp-server split maps onto it one-to-one.
2. **Ecosystem naming coherence**: `mif-rh*` names the crates by what they
   are — MIF tooling for the research harness — rather than introducing a
   second, unrelated brand (`hoe`) into the MIF ecosystem.

## Considered Options

### Option 1: Standalone harness-ontology-engine repository (the RFC's proposal)

**Description**: A new repository/workspace `harness-ontology-engine` with
crates `engine-core`/`cli`/`mcp-server`, one `hoe` binary with three
subcommands, as proposed in `rust-rfc-engine-core.md:76-109`.

**Advantages**:

- Matches the published RFC text exactly; no naming mismatch for readers of
  the design-doc set.
- Independent versioning and release cadence from the rest of the MIF Rust
  ecosystem.

**Disadvantages**:

- Duplicates the entire CI/lint/deny/release/attestation stack for a
  proof-of-concept-phase project.
- Path dependencies on `mif-ontology`/`mif-embed`/`mif-problem` become
  cross-repo version dependencies; a breaking library change lands in two
  repos and two PRs instead of one.
- A single `hoe` binary bundles MCP-server dependencies into the CLI and
  vice versa, where two thin binaries keep each dependency tree minimal.

**Risk Assessment**:

- **Technical Risk**: Low. It would work; it is simply more moving parts.
- **Schedule Risk**: Medium. Standing up a new attested repository is real
  work the proof-of-concept phase does not need.
- **Ecosystem Risk**: Medium. A second repo and brand to maintain, document,
  and eventually deprecate if the proof of concept does not graduate.

### Option 2: mif-rh crates inside the mif-rs workspace (chosen)

**Description**: Three crates — `mif-rh` (engine library), `mif-rh-cli`,
`mif-rh-mcp` — as members of this virtual workspace, producing two
independent binaries. The RFC's engine-core/CLI/MCP separation is preserved;
only naming and packaging differ.

**Advantages**:

- Real path dependencies on the library crates; one PR catches a breaking
  change across the whole dependency chain.
- Workspace lints, `cargo deny`, CI, release, and attestation infrastructure
  inherited for free — the release workflows already scale to any number of
  members and `[[bin]]` targets by design.
- Two thin binaries keep the CLI free of MCP wire-protocol dependencies and
  the MCP server free of `clap`, matching ADR-0004's thin-consumer pattern.

**Disadvantages**:

- Diverges from the RFC's published naming; requires exactly the record this
  ADR provides.
- The engine's release cadence is coupled to the workspace's (all members
  version and release together).

**Risk Assessment**:

- **Technical Risk**: Low. The shape is this workspace's established pattern.
- **Schedule Risk**: Low. No new infrastructure to build.
- **Ecosystem Risk**: Low. One repository, one brand, one release pipeline.

### Option 3: Vendor the engine into research-harness-template itself

**Description**: Build the Rust engine inside the harness template repository
it serves, alongside the bash scripts it reimplements.

**Advantages**:

- The engine and its reference implementation live and version together;
  parity fixtures are always at hand.

**Disadvantages**:

- `research-harness-template` is a Copier template consumed by instances; a
  Rust toolchain, workspace, and release pipeline inside it would bloat every
  instance and entangle template updates with engine releases.
- The engine's library layer is genuinely reusable MIF tooling; burying it in
  a template repo hides it from every other MIF consumer.

**Risk Assessment**:

- **Technical Risk**: Medium. Copier-template mechanics and Cargo workspaces
  interact poorly (`.jinja` suffix rendering, gitignored build artifacts).
- **Schedule Risk**: Medium.
- **Ecosystem Risk**: High. Couples two release lifecycles that have no
  reason to be coupled.

## Decision

We package the compiled research-harness ontology engine as **three crates
inside the mif-rs workspace** — `mif-rh` (library), `mif-rh-cli` and
`mif-rh-mcp` (thin binaries) — and do not create a standalone
`harness-ontology-engine` repository or a unified `hoe` binary. The RFC's
engine-core/CLI/MCP separation of concerns is realized by the crate
boundaries; `mif-rh` depends on no CLI or MCP wire-protocol crate.

### Intentional behavioral divergence from the bash reference

One divergence from the bash scripts is recorded here as deliberate, not
accidental (gap analysis Risk R-1): `mif_rh::review` runs
`check-relationship-targets.sh` exactly **once** per review call, corpus-wide
(`crates/mif-rh/src/review.rs:298-329`). The `research-harness` instance
branch that served as this port's reference snapshot invokes the script
**twice** in corpus-wide mode (`scripts/ontology-review.sh` lines 169 and
196 on branch `mif-rh-m1-parity-testing` — the second block re-initializes
its own result flag and duplicates the first block's comment, an
unintentional duplication). `research-harness-template`'s current HEAD
invokes it once. The engine's single invocation therefore matches the
template's current behavior and is the correct one; byte-for-byte parity
with the duplicated variant is explicitly not a goal.

## Consequences

### Positive

1. **One PR spans the whole chain**: a breaking change in `mif-ontology` or
   `mif-embed` and its mif-rh fallout are visible and fixable atomically.
2. **Full quality-gate inheritance**: pedantic clippy, `cargo deny`,
   SHA-pinned CI, signed release attestations — all apply to the engine from
   its first commit.

### Negative

1. **Naming mismatch against the published RFC**: permanent, and mitigated
   only by records like this one (the workspace gap analysis's glossary
   makes the same note; the RFC itself carries no such correction).
2. **Coupled release cadence**: the engine cannot ship a release without the
   workspace shipping one.

### Neutral

1. **Two binaries instead of one**: callers invoke `mif-rh-cli` or
   `mif-rh-mcp` directly; there is no `hoe mcp-serve`. The bash-fallback
   contract (NFR-4) is unaffected — callers opt in by binary name either way.

## Decision Outcome

The decision achieves its objective — a compiled engine with the RFC's
internal architecture, at zero new infrastructure cost — measured by:
`mif-rh` carries no `clap` or MCP wire-protocol dependency
(`crates/mif-rh/Cargo.toml`); both binaries build and release through the
existing `cargo metadata`-driven pipeline with no workflow changes; and all
workspace quality gates run over the three crates identically to every other
member.

### Open question: two lock-staleness models coexist (gap analysis R-10)

`mif-rh`'s `ReviewLock` guards one `review` run with a native advisory file
lock plus **PID-liveness** staleness detection (`crates/mif-rh/src/lock.rs`).
The harness's own pre-existing `run-lock.sh` guards a whole research run at
topic scope with an atomically-`mkdir`ed lock directory whose staleness is a
**time window** (directory mtime, `RUN_LOCK_STALE_MIN`, default 240 minutes,
refreshed at phase boundaries). These are different mechanisms for different
scopes and currently coexist without conflict. Any future generalization that
folds harness-lifecycle locking into the engine (the deferred M4 decision)
must reconcile the two models rather than assume one subsumes the other: a
PID-liveness check would misclassify a live-but-slow multi-hour research run
held across process boundaries, which the time-window model tolerates by
design. This is an open design question for M4, not a defect in either
implementation.

## Related Decisions

- [ADR-0003: Virtual Cargo Workspace, Not a Root Package](https://modeled-information-format.github.io/mif-rs/adr/0003-virtual-cargo-workspace/) — the workspace this decision extends to three more members.
- [ADR-0004: Library Crates Never Depend on the Binary Crates](https://modeled-information-format.github.io/mif-rs/adr/0004-libraries-never-depend-on-binaries/) — the layering pattern the mif-rh crates follow.
- [ADR-0020: mif-rh-mcp Speaks stdio-Only MCP Transport](https://modeled-information-format.github.io/mif-rs/adr/0020-mif-rh-mcp-stdio-only-transport/) — the companion record closing the same RFC's transport question.

## Links

- research-harness-template ADR-0014, "Compiled ontology engine as a scoped CLI+MCP proof-of-concept" — the authorizing decision.
- `research-harness-template` `docs/proposals/ontology-engine/rust-rfc-engine-core.md` (lines 76-109) — the RFC's proposed `harness-ontology-engine`/`hoe` shape this decision deviates from.
- `mif-rh-punchlist.md` (workspace gap analysis, 2026-07-04; an unpublished, workspace-local analysis cited for provenance — not resolvable from this published record) — Risks R-1, R-8, and R-10, all closed or recorded by this ADR.

## More Information

- **Date**: 2026-07-04
- **Source**: reconstructed from the shipped crate layout (workspace
  `Cargo.toml` members, `crates/mif-rh*/`), the RFC text, and the 2026-07-04
  gap analysis. No original packaging decision record exists; this ADR is the
  retroactive record, labeled as such.

## Audit

### 2026-07-04

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| Three mif-rh crates are workspace members; no standalone repo, no `hoe` binary | Cargo.toml | members list | accepted |
| `mif-rh` has no clap/MCP wire-protocol dependency | crates/mif-rh/Cargo.toml | dependencies | accepted |
| Single corpus-wide relationship-script invocation | crates/mif-rh/src/review.rs | 298-329 | accepted, intentional divergence recorded |
| Reference-snapshot double invocation exists on the instance branch, not template HEAD | research-harness `scripts/ontology-review.sh` (branch mif-rh-m1-parity-testing) | 169, 196 | verified 2026-07-04 |
| Two lock-staleness models (PID-liveness vs time-window) coexist, unreconciled | crates/mif-rh/src/lock.rs; research-harness-template scripts/run-lock.sh | — | open question for M4, recorded |

**Summary:** The packaging described is what exists on `main`. The
Considered Options section is reconstructed rationale, disclosed as such,
matching the precedent ADR-0015 set for retroactive records.

**Action Required:** None.
