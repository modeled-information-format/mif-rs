---
mode: ask
description: Generate unit tests and proptest property tests for a module
---

# Write Tests

Generate comprehensive tests for a module or function.

## Inputs

- **Target**: Module or function to test (e.g., `crates/parser.rs`)

## Steps

1. Read the target source file and identify all public functions
2. For each public function, create unit tests covering:
   - **Happy path**: Valid inputs produce expected outputs
   - **Error cases**: Invalid inputs return appropriate errors
   - **Edge cases**: Empty inputs, boundary values, special characters
3. Add property-based tests using `proptest` where applicable:
   - Roundtrip properties (encode/decode, serialize/deserialize)
   - Invariant properties (output always satisfies condition)
   - Commutativity or associativity where relevant
   - Note: `proptest` is not currently a workspace dependency — add it to the
     target crate's `Cargo.toml` first if a property test is warranted
4. Place unit tests in `#[cfg(test)] mod tests` inside the source file
5. Place integration tests in `crates/<name>/tests/` (per-crate) if they test
   cross-module behavior — this workspace has no shared top-level `tests/`
   directory
6. Use descriptive names: `test_<function>_<scenario>_<expected>`
7. Run `cargo test --all-features` to verify all tests pass
