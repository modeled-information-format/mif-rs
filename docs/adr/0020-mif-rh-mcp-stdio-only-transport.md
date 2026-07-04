---
title: "mif-rh-mcp Speaks stdio-Only MCP Transport"
description: "Expose the mif-rh MCP server over stdio only (rmcp with the transport-io feature), wiring no HTTP or SSE transport, and record this as the answer to the authorizing RFC's unresolved wire-protocol question."
type: adr
conceptType: semantic
x-ontology:
  id: mif-docs
  version: "1.0.0"
  entity_type: decision-record
category: architecture
tags:
  - adr
  - mcp
  - transport
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
  - 0019-mif-rh-crates-in-mif-rs-workspace.md
---

# ADR-0020: mif-rh-mcp Speaks stdio-Only MCP Transport

## Status

Accepted

## Context

### Background and Problem Statement

The RFC authorizing the compiled ontology engine
(`research-harness-template`'s
`docs/proposals/ontology-engine/rust-rfc-engine-core.md`, lines 200-204) left
the MCP wire-protocol crate choice as an explicit Unresolved Question. The
shipped implementation answers it in code: `mif-rh-mcp` uses `rmcp` with
exactly the `server` and `transport-io` features
(`crates/mif-rh-mcp/Cargo.toml`) and serves over stdio
(`crates/mif-rh-mcp/src/main.rs` — `use rmcp::transport::stdio;`,
`MifRh.serve(stdio())`), mirroring `mif-mcp`. No HTTP, SSE, or
streamable-HTTP transport is wired.

Nothing recorded that this closes the RFC's question, nor whether stdio-only
is sufficient for every intended consumer. A 2026-07-04 architecture gap
analysis (`mif-rh-punchlist.md`, Risk R-9) flagged the gap: a future consumer
needing a network-reachable MCP server would discover the limitation only by
reading source. This ADR is that record.

### Current Limitations

1. **The RFC's Unresolved Question was answered implicitly**: the crate and
   transport choice existed only as code, invisible to readers of the
   design-doc set.
2. **No consumer inventory existed**: sufficiency of stdio was assumed, not
   stated against a list of known consumers.

## Decision Drivers

### Primary Decision Drivers

1. **Every known consumer spawns the server locally.** The MCP tools exist
   for agents working inside a research-harness corpus checkout (Claude Code
   and similar agent hosts), all of which launch MCP servers as local stdio
   subprocesses. WHEN an agent host configures `mif-rh-mcp` as a stdio
   server, THE SYSTEM SHALL serve all four tools with no further transport
   configuration.
2. **No authentication story is needed for stdio.** A network transport
   requires authentication, TLS, and a hardening posture that a
   local-subprocess tool does not; shipping a network listener without those
   would be worse than shipping no listener.

### Secondary Decision Drivers

1. **Smaller dependency and attack surface**: `transport-io` alone keeps
   axum/hyper-class HTTP dependencies out of the binary entirely, which also
   keeps the `cargo deny` surface and release-artifact size down.
2. **Consistency**: `mif-mcp` (this workspace's other MCP server) is already
   stdio-only; two servers with matching transport contracts are easier to
   document and operate.

## Considered Options

### Option 1: stdio-only via rmcp transport-io (chosen)

**Description**: Serve MCP over stdin/stdout only, using `rmcp`'s
`transport-io` feature. The server is always spawned as a subprocess by its
consumer.

**Advantages**:

- Zero transport configuration, zero authentication surface, minimal
  dependency tree.
- Matches how every currently known consumer actually launches MCP servers.

**Disadvantages**:

- A consumer needing a shared, network-reachable server (one corpus, many
  remote agents) is unsupported until a superseding decision adds a
  transport.

**Risk Assessment**:

- **Technical Risk**: Low. The transport is the simplest one the protocol
  defines, and `rmcp` maintains it as a first-class feature.
- **Schedule Risk**: Low. It is what already ships.
- **Ecosystem Risk**: Low, bounded by the revisit trigger below.

### Option 2: stdio plus a network transport (streamable HTTP or SSE)

**Description**: Wire `rmcp`'s HTTP-class transport features alongside stdio
and add a `--listen` mode to the binary.

**Advantages**:

- Supports remote or shared-server topologies from day one.

**Disadvantages**:

- Requires an authentication/TLS/hardening design no current consumer needs.
- Pulls an HTTP stack into the dependency tree, enlarging the supply-chain
  and audit surface for speculative benefit — exactly the "flexibility
  nobody asked for" this workspace's conventions reject.

**Risk Assessment**:

- **Technical Risk**: Medium. Unexercised transport code paths rot.
- **Schedule Risk**: Medium.
- **Ecosystem Risk**: Medium. A network listener in a security-attested
  release artifact demands ongoing scrutiny.

### Option 3: Network-only

**Description**: Drop stdio; serve exclusively over a network transport.

**Advantages**:

- One transport code path, suited to a centralized-service deployment model.

**Disadvantages**:

- Breaks every known consumer, all of which spawn stdio subprocesses.
- Inherits all of Option 2's authentication and hardening costs while
  removing the mode that currently works.

**Risk Assessment**:

- **Technical Risk**: High for current consumers (none could connect).
- **Schedule Risk**: Medium.
- **Ecosystem Risk**: High.

## Decision

`mif-rh-mcp` serves MCP over **stdio only**, via `rmcp` with the `server` and
`transport-io` features. This ADR closes the RFC's Unresolved Question on the
wire-protocol crate: the answer is `rmcp`, stdio transport, and it is
sufficient for all currently known consumers.

## Consequences

### Positive

1. **No authentication or TLS surface** in a release-attested binary.
2. **Minimal dependency tree**: no HTTP stack in `mif-rh-mcp`'s `cargo deny`
   or SBOM footprint.

### Negative

1. **No shared-server topology**: multiple agents on one corpus each spawn
   their own process. Acceptable at current scale; the index is read-only for
   MCP consumers, so concurrent readers do not conflict.

### Neutral

1. **The choice is reversible by supersession**: adding a transport later is
   additive to the binary's interface and would arrive with its own ADR.

## Decision Outcome

The decision meets its objective — a working MCP server for local agent
hosts with the smallest possible surface — measured by: the four tools
(`search`, `suggest_type`, `find_similar`, `corpus_stats`) are reachable from
a stdio-configured agent host with no transport flags, and
`crates/mif-rh-mcp/Cargo.toml` carries no HTTP-transport feature or
dependency.

**Revisit trigger**: the first real consumer that needs a network-reachable
`mif-rh-mcp` (a shared corpus server, a remote-agent deployment) supersedes
this ADR with a transport-addition decision that includes an authentication
design — the transport must not be added without one.

## Related Decisions

- [ADR-0019: Ship the Research-Harness Ontology Engine as mif-rh Crates Inside the mif-rs Workspace](https://modeled-information-format.github.io/mif-rs/adr/0019-mif-rh-crates-in-mif-rs-workspace/) — the companion record for the same RFC's packaging deviation.

## Links

- `research-harness-template` `docs/proposals/ontology-engine/rust-rfc-engine-core.md` (lines 200-204) — the Unresolved Question this ADR closes.
- [rmcp](https://crates.io/crates/rmcp) — the MCP SDK crate; `mif-rh-mcp` enables its `server` and `transport-io` features only.
- `mif-rh-punchlist.md` (workspace gap analysis, 2026-07-04) — Risk R-9, closed by this ADR.

## More Information

- **Date**: 2026-07-04
- **Source**: `crates/mif-rh-mcp/src/main.rs` and `Cargo.toml` (the shipped
  transport wiring); the RFC's Unresolved Questions section. Written
  retroactively; the rationale is reconstructed from the shipped code and
  this workspace's stated conventions, labeled as such.

## Audit

### 2026-07-04

**Status:** Compliant

**Findings:**

| Finding | Files | Lines | Assessment |
| --- | --- | --- | --- |
| stdio transport wiring, no other transport present | crates/mif-rh-mcp/src/main.rs | transport import + serve call | accepted |
| rmcp features limited to server, transport-io | crates/mif-rh-mcp/Cargo.toml | dependencies | accepted |

**Summary:** The shipped transport is stdio-only as described; this ADR
records the implicit decision and its revisit trigger.

**Action Required:** None.
