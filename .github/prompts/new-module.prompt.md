---
mode: ask
description: Scaffold a new Rust module with lib.rs export, tests, and documentation
---

# New Module

Create a new Rust module for this project.

## Inputs

- **Module name**: The name for the new module (snake_case)
- **Purpose**: Brief description of what the module does

## Steps

1. Pick the crate the module belongs to, respecting the workspace's dependency
   chain: `mif-core` has no internal deps; `mif-schema` depends only on
   `mif-core`; `mif-ontology` depends on both; `mif-cli`/`mif-mcp` depend on
   whichever libraries they call, directly.
2. Create `crates/{crate_name}/src/{module_name}.rs` with:
   - Module-level doc comment explaining purpose
   - Public types and functions with full documentation
   - `# Examples` and `# Errors` sections on all public items
   - Error type using `thiserror` if the module has fallible operations
3. Add `pub mod {module_name};` to `crates/{crate_name}/src/lib.rs`
4. Re-export key public items from `crates/{crate_name}/src/lib.rs` if appropriate
5. If the module introduces a new error enum in a crate whose existing errors
   implement `mif_problem::ToProblem` (`mif-schema`, `mif-ontology`,
   `mif-frontmatter`, `mif-embed`, `mif-store`, `mif-cli`'s `CliError`,
   `mif-mcp`'s `McpError`), implement `ToProblem` for it too, with a match arm
   per variant — the match is exhaustive, so a missing arm fails to compile
6. Add a `#[cfg(test)] mod tests` block with:
   - At least one success-path test
   - At least one error-path test
7. Ensure no `unwrap()`, `expect()`, or `panic!()` in the module
8. Run `cargo clippy --all-targets --all-features -- -D warnings`
9. Run `cargo test`
