---
applyTo: "crates/**/tests/**/*.rs,crates/**/src/**/*.rs"
---

# Test Instructions

When generating or modifying test files — integration tests under
`crates/<name>/tests/` (e.g. `crates/mif-cli/tests/exit_codes.rs`, the only
one that exists today) or `#[cfg(test)] mod tests` blocks inside
`crates/**/src/**/*.rs` (the dominant test location in this workspace):

## Test Structure

- Use descriptive test names: `test_<function>_<scenario>_<expected>`
- Group related tests in modules
- Use `assert_eq!` for equality, `assert!(matches!(...))` for patterns
- Test both success and error paths

## Property-Based Testing

`proptest` is not currently a workspace dependency. If a test genuinely
warrants property-based testing, add `proptest` to the relevant crate's
`Cargo.toml` first, then:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn property_name(input in strategy()) {
        prop_assert!(condition(input));
    }
}
```

## Test Helpers

- Integration tests live per-crate at `crates/<name>/tests/`, not a shared
  top-level `tests/` directory
- Use `#[cfg(test)]` for unit test modules inside source files

## Assertions

- Prefer `assert_eq!(actual, expected)` over `assert!(actual == expected)`
- Use `assert!(matches!(result, Err(Error::Variant(_))))` for error matching
- Include descriptive messages: `assert_eq!(a, b, "values should match after transform")`

## No Panics in Test Setup

- Test assertions may panic (that is their purpose)
- Test setup and teardown should use `Result` where possible
- Use `#[test] fn test_name() -> Result<(), Error>` for fallible tests
