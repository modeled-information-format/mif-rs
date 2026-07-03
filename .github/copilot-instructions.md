# GitHub Copilot Instructions

This document provides context for GitHub Copilot when working with this Rust project.

## Project Context

This is a Rust Cargo workspace using modern tooling:
- **Rust**: 1.92+ (2024 edition)
- **Build System**: Cargo (virtual workspace, `crates/*` members)
- **Linting**: clippy with the pedantic, nursery, and cargo lint groups; `unsafe_code = "forbid"` workspace-wide
- **Formatting**: rustfmt
- **Testing**: Built-in test framework (unit tests + doc tests); `proptest` is not currently a workspace dependency
- **Supply Chain Security**: cargo-deny
- **Error reporting**: RFC 9457 Problem Details via the shared `mif-problem` crate — see "Error Handling" below

## Code Generation Guidelines

### Error Handling

Use `Result` types instead of panicking:

```rust
// Good - Returns Result
pub fn parse_value(input: &str) -> Result<i64, ParseError> {
    input.parse().map_err(|e| ParseError::InvalidFormat(e))
}

// Avoid - Panics on failure
pub fn parse_value(input: &str) -> i64 {
    input.parse().unwrap() // Never do this
}
```

Use `thiserror` for custom error types:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("operation failed: {operation}")]
    OperationFailed { operation: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

Most crates' error enums additionally implement `mif_problem::ToProblem`, mapping
each variant to an RFC 9457 `application/problem+json` envelope (`ProblemDetails`)
via a per-variant `ProblemMeta`. This match is exhaustive — adding a new error
variant to one of these enums (`mif-schema`, `mif-ontology`, `mif-frontmatter`,
`mif-embed`, `mif-store`, `mif-cli`'s `CliError`, `mif-mcp`'s `McpError`) requires
adding a corresponding arm in `to_problem()` (and typically `meta()`), or the
crate fails to compile with a non-exhaustive-match error.

### Type Annotations

Provide explicit return types for public functions:

```rust
// Good - explicit return type
pub fn process_data(items: &[String]) -> Vec<ProcessedItem> {
    // implementation
}

// Avoid for public APIs
pub fn process_data(items: &[String]) -> impl Iterator<Item = ProcessedItem> {
    // implementation
}
```

### Documentation

Use doc comments with examples:

```rust
/// Processes a list of items according to the given configuration.
///
/// # Arguments
///
/// * `items` - The items to process.
/// * `config` - Configuration options for processing.
///
/// # Returns
///
/// A vector of processed items.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if any item is invalid.
///
/// # Examples
///
/// ```rust
/// use mif_core::{process, Config};
///
/// let items = vec!["a", "b", "c"];
/// let config = Config::default();
/// let result = process(&items, &config)?;
/// # Ok::<(), mif_core::Error>(())
/// ```
pub fn process(items: &[&str], config: &Config) -> Result<Vec<Item>> {
    // implementation
}
```

### Ownership and Borrowing

Prefer borrowing over ownership when possible:

```rust
// Good - borrows the slice
pub fn sum_values(values: &[i64]) -> i64 {
    values.iter().sum()
}

// Avoid - takes ownership unnecessarily
pub fn sum_values(values: Vec<i64>) -> i64 {
    values.iter().sum()
}
```

Use `Cow` for efficient string handling:

```rust
use std::borrow::Cow;

pub fn normalize_name(name: &str) -> Cow<'_, str> {
    if name.contains(' ') {
        Cow::Owned(name.replace(' ', "_"))
    } else {
        Cow::Borrowed(name)
    }
}
```

### Structs and Enums

Use builder pattern for complex structs:

```rust
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub timeout: Duration,
    pub retries: u32,
    pub verbose: bool,
}

impl Config {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }
}
```

### Testing

Write comprehensive unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_positive_numbers() {
        assert_eq!(add(2, 3), 5);
    }

    #[test]
    fn test_divide_by_zero_returns_error() {
        let result = divide(10, 0);
        assert!(matches!(result, Err(Error::DivisionByZero)));
    }
}
```

`proptest` is not currently a workspace dependency — do not add property-based
tests without first adding `proptest` as an explicit dependency for the crate
that needs it.

### Async Code

Only `mif-mcp` uses async Rust, for its MCP server (`rmcp` with the
`transport-io` feature) over stdio — via `tokio` (`rt-multi-thread`, `macros`,
`io-std` features). There is no outbound async HTTP client (`reqwest` is not a
workspace dependency); don't introduce one without an explicit reason. The
library crates (`mif-core`, `mif-schema`, `mif-ontology`, `mif-problem`,
`mif-frontmatter`, `mif-embed`, `mif-store`) and `mif-cli` are synchronous.

## Common Patterns

### Iterator Chains

```rust
let result: Vec<_> = items
    .iter()
    .filter(|item| item.is_valid())
    .map(|item| item.transform())
    .collect();
```

### Option and Result Combinators

```rust
// Option chaining
let value = config
    .get_setting("key")
    .and_then(|s| s.parse().ok())
    .unwrap_or_default();

// Result chaining with ?
fn process() -> Result<Output> {
    let data = load_data()?;
    let parsed = parse_data(&data)?;
    let result = transform(parsed)?;
    Ok(result)
}
```

### Const Functions

Prefer `const fn` for compile-time evaluation:

```rust
#[must_use]
pub const fn new() -> Self {
    Self { value: 0 }
}
```

## File Locations

- Source code: `crates/<crate-name>/src/`
- Library entry: `crates/<crate-name>/src/lib.rs`
- Binary entry: `crates/<crate-name>/src/main.rs`
- Integration tests: `crates/<crate-name>/tests/` (e.g. `crates/mif-cli/tests/`)
- Unit and doc tests: `#[cfg(test)] mod tests` in each source file, and `///` doc examples
- No `benches/` directory or `criterion` dependency currently exists in this workspace

## Commands

```bash
cargo build           # Build
cargo test            # Run tests
cargo clippy          # Lint
cargo fmt             # Format
cargo doc --open      # Generate and view docs
cargo deny check      # Check supply chain
```
