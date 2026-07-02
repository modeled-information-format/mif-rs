---
id: how-to-add-property-based-test-mif-rs
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/testing
title: How to Add a Property-Based Test to an mif-rs Crate
tags:
  - how-to
  - testing
  - proptest
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
  name: Add a Property-Based Test to an mif-rs Crate
  entity_type: how-to-guide
---

# How to Add a Property-Based Test to an mif-rs Crate

Add a property-based test to a workspace crate when you need to verify an
invariant holds across generated inputs, not just a handful of hand-picked
examples — for example, that `mif-ontology::resolve_chain` always returns a
chain no longer than the corpus it resolved against. `proptest` is not
currently a workspace dependency, so this guide starts by adding it.

## Prerequisites

- A workspace member crate with existing `#[cfg(test)] mod tests` coverage
  (this guide uses `mif-ontology` as the target crate).
- `just check` passing on `main` before you start.

## Step 1 — Add `proptest` to the workspace dependency table

Add it once, in the root `Cargo.toml`'s `[workspace.dependencies]` table, so
every crate references the same pinned version:

```toml
# Cargo.toml
[workspace.dependencies]
proptest = "1.11.0"
```

## Step 2 — Add `proptest` as a dev-dependency of the target crate

```toml
# crates/mif-ontology/Cargo.toml
[dev-dependencies]
proptest.workspace = true
```

## Step 3 — Write the property test

Add a `property_tests` submodule inside the crate's existing
`#[cfg(test)] mod tests` block in `crates/mif-ontology/src/lib.rs`. This
example checks that a linear `extends` chain of any depth resolves in
base-to-specific order with no entries dropped or duplicated:

```rust
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashMap;

    fn metadata(id: &str, extends: Vec<String>) -> OntologyMetadata {
        OntologyMetadata {
            id: id.to_string(),
            version: "1.0.0".to_string(),
            description: None,
            extends,
        }
    }

    proptest! {
        #[test]
        fn linear_chain_resolves_in_order(depth in 1usize..8) {
            let mut corpus = HashMap::new();
            for i in 0..depth {
                let id = format!("tier-{i}");
                let extends = if i == 0 { vec![] } else { vec![format!("tier-{}", i - 1)] };
                corpus.insert(id.clone(), metadata(&id, extends));
            }
            let target = format!("tier-{}", depth - 1);

            let chain = resolve_chain(&target, &corpus).unwrap();

            prop_assert_eq!(chain.len(), depth);
            prop_assert_eq!(&chain.first().unwrap().id, "tier-0");
            prop_assert_eq!(&chain.last().unwrap().id, &target);
        }
    }
}
```

`.unwrap()` is fine here: `clippy.toml`'s `allow-unwrap-in-tests = true`
exempts `#[cfg(test)]` code from the workspace's denied `unwrap_used` lint.

## Step 4 — Run the property test

```bash
cargo test -p mif-ontology linear_chain_resolves_in_order
```

`proptest` runs the test against 256 generated `depth` values by default,
shrinking automatically to a minimal failing case if any input breaks the
invariant.

## Step 5 — Confirm the full crate still passes

```bash
cargo test -p mif-ontology --all-features
```

The property test is in place and running as part of the crate's normal test
suite.
