---
mode: ask
description: Add a new thiserror variant with proper attributes
---

# Add Error Variant

Add a new variant to an existing `thiserror` error enum.

## Inputs

- **Error enum**: Which error type to extend (e.g., `Error` in `crates/error.rs`)
- **Variant name**: Name for the new variant (PascalCase)
- **Message**: Human-readable error message for `#[error(...)]`
- **Fields**: Any fields the variant should carry

## Steps

1. Add the new variant to the error enum with `#[error("...")]` attribute
2. If wrapping another error, use `#[from]` or `#[source]` as appropriate:
   - `#[from]` for automatic conversion (one per source type)
   - `#[source]` for manual conversion or multiple variants from same type
3. Update any match arms that handle this error enum exhaustively
4. If the crate's error enum implements `ToProblem` (`mif-schema`, `mif-ontology`, `mif-frontmatter`, `mif-embed`, `mif-store`, `mif-cli`'s `CliError`, and `mif-mcp`'s `McpError` all do), you must also add a corresponding match arm in `to_problem()` (and typically `meta()`) for the new variant — the match is exhaustive, so omitting this fails to compile
5. Add a unit test verifying the error Display output
6. Update doc comments on functions that can now return this variant
7. Run `cargo clippy --all-targets --all-features -- -D warnings`
8. Run `cargo test --all-features`
