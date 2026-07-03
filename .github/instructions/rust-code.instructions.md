---
applyTo: "crates/**/*.rs"
---

# Rust Code Instructions

When generating or modifying Rust source files in `crates/`:

## Error Handling

- Return `Result` types for all fallible operations
- Use `thiserror` for custom error types with `#[error(...)]` attributes
- Propagate errors with `?` operator
- Never use `unwrap()`, `expect()`, or `panic!()` in library code
- Most crates' error enums implement `mif_problem::ToProblem`, mapping each variant to an RFC 9457 `ProblemDetails` envelope via a per-variant `ProblemMeta` — the `to_problem()` match is exhaustive, so a new variant on one of these enums needs a corresponding match arm there too, or the crate fails to compile

## Ownership and Borrowing

- Prefer `&str` over `String` in function parameters
- Prefer `&[T]` over `Vec<T>` in function parameters
- Use `Cow<'_, str>` for flexible string returns
- Use `Vec::with_capacity()` when size is known

## Functions

- Use `const fn` where possible
- Add `#[must_use]` to functions returning values that should not be ignored
- Prefer `impl Trait` for private return types, explicit types for public APIs

## Documentation

- All public items require doc comments
- Include `# Arguments`, `# Returns`, `# Errors`, and `# Examples` sections
- Examples must compile and run as doc tests
- Use `# Ok::<(), crate::Error>(())` to handle Results in doc tests

## Style

- Maximum line length: 100 characters
- Edition 2024 idioms
- Group imports: std, external crates, crate-local
- Use `imports_granularity = "Crate"` style
