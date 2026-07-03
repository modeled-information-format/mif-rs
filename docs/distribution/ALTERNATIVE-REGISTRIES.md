---
id: how-to-publish-crate-to-alternative-registry
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/distribution
title: How to Publish an mif-rs Crate to an Alternative Cargo Registry
tags:
  - how-to
  - distribution
  - cargo
  - registry
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-02T00:00:00Z'
  recordedAt: '2026-07-02T00:00:00Z'
  ttl: P1Y
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: Publish an mif-rs Crate to an Alternative Cargo Registry
  entity_type: how-to-guide
---

# How to Publish an mif-rs Crate to an Alternative Cargo Registry

Publish one of the workspace's library crates (`mif-core`, `mif-schema`,
`mif-ontology`, `mif-problem`, `mif-frontmatter`, `mif-embed`, or `mif-store`)
to a private or internal sparse-index registry instead of, or in addition to,
crates.io — for example, when running an internal fork that cannot depend on
the public registry. This guide uses `mif-core` and a registry named
`internal` as the concrete example; substitute your own crate and registry
name throughout.

## Prerequisites

- A running Cargo sparse-index registry (any implementation — the registry
  index URL and an auth token are all Cargo needs) and its sparse index URL.
- A publish token for that registry.
- Write access to the crate's `Cargo.toml` and to `~/.cargo/config.toml` or
  the project's `.cargo/config.toml`.

## Step 1 — Register the alternative registry with Cargo

Add the registry's sparse index to `~/.cargo/config.toml` (or
`.cargo/config.toml` at the workspace root to scope it to this project):

```toml
[registries.internal]
index = "sparse+https://registry.example.com/index/"
```

## Step 2 — Authenticate to the registry

```bash
cargo login --registry internal
```

This stores the token from your registry's credential page in Cargo's
credential store; it is never checked into `Cargo.toml`.

## Step 3 — (Optional) Restrict which registries the crate may publish to

None of the workspace's crates set a `publish` field in their `[package]`
table today — all 9 have already published to crates.io at version 0.1.0.
With `publish` unset, Cargo's default (`publish = true`) leaves every crate
free to publish to any registry, including `internal`, with no `Cargo.toml`
change required — skip straight to Step 4.

Only add an explicit allow-list if you want to *restrict* a crate to a
specific set of registries (for example, to prevent an internal-only crate
from ever being published to crates.io). Note that naming an allow-list
excludes crates.io unless you include it explicitly — since this workspace's
crates are already public on crates.io, adding one here would need to
include `"crates-io"` to avoid blocking future public releases:

```toml
# crates/mif-core/Cargo.toml
[package]
publish = ["crates-io", "internal"]
```

**Note on republishing to crates.io itself:** crates.io versions are
immutable — `mif-cli`, `mif-mcp`, `mif-schema`, and `mif-ontology` have each
gained functionality since their initial 0.1.0 publish (new CLI subcommands
and MCP tools, for example). Getting that new functionality onto crates.io is
not a `publish`-field change at all; it requires bumping `version` in the
workspace's `Cargo.toml` and running the normal release/publish pipeline (see
[RELEASING.md](../runbooks/RELEASING.md)) — a new version, not a new flag.

## Step 4 — Publish

Run from the workspace root, targeting the crate by name:

```bash
cargo publish -p mif-core --registry internal
```

Cargo verifies the package builds in isolation before uploading it.

## Step 5 — Verify the crate is resolvable from the registry

From a separate project configured with the same `[registries.internal]`
entry:

```bash
cargo add mif-core --registry internal
```

The crate resolves and downloads from the alternative registry, confirming
the publish succeeded.
