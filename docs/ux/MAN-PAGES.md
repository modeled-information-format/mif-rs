---
id: how-to-generate-man-pages-mif-cli
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/cli
title: How to Generate Man Pages for mif-cli
tags:
  - how-to
  - cli
  - man-pages
  - mif-cli
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
  name: Generate Man Pages for mif-cli
  entity_type: how-to-guide
---

# How to Generate Man Pages for mif-cli

Produce a Unix manual page (`mif-cli.1`) at build time from the `mif-cli`
binary's `clap` definition, for local installation or downstream packaging
(`.deb`, `.rpm`, Homebrew).

## Prerequisites

- The `mif-rs` workspace checked out, with `cargo build -p mif-cli` working.
- `crates/mif-cli/src/main.rs` defines `Cli` (via `clap::Parser`) with the
  `Command` enum (`Validate`, `Ontology { command: OntologyCommand }`) and
  `OntologyCommand::Resolve`.

## Step 1 — Extract the CLI definition into its own module

`build.rs` runs as a separate compilation unit and cannot import private
items from `main.rs`. Move the `Cli`, `Command`, and `OntologyCommand`
definitions into `crates/mif-cli/src/cli.rs`, making them `pub`:

```rust
// crates/mif-cli/src/cli.rs
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mif-cli",
    version,
    about = "CLI for the MIF (Modeled Information Format) ecosystem"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Validate a MIF document against the canonical schema.
    Validate {
        /// Path to the MIF document (JSON-LD projection) to validate.
        file: PathBuf,
    },
    /// Ontology-related operations.
    Ontology {
        #[command(subcommand)]
        command: OntologyCommand,
    },
}

#[derive(Subcommand)]
pub enum OntologyCommand {
    /// Resolve an ontology's three-tier `extends` chain.
    Resolve {
        /// The ontology ID to resolve.
        id: String,
        /// Directory containing ontology definition YAML files.
        #[arg(long)]
        ontologies_dir: PathBuf,
    },
}
```

In `crates/mif-cli/src/main.rs`, replace the inline `Cli`/`Command`/
`OntologyCommand` definitions with a module declaration and import:

```rust
mod cli;
use cli::{Cli, Command, OntologyCommand};
```

## Step 2 — Add clap_mangen as a build dependency

Edit `crates/mif-cli/Cargo.toml`:

```toml
[build-dependencies]
clap = { version = "4.6.1", features = ["derive"] }
clap_mangen = "0.3"
```

## Step 3 — Write build.rs to render the man pages

```rust
// crates/mif-cli/build.rs
use std::env;
use std::fs;
use std::path::PathBuf;

use clap::CommandFactory;
use clap_mangen::Man;

include!("src/cli.rs");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/cli.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR not set")?);
    let man_dir = out_dir.join("man");
    fs::create_dir_all(&man_dir)?;

    let cmd = Cli::command();

    let mut buffer = Vec::new();
    Man::new(cmd.clone()).render(&mut buffer)?;
    fs::write(man_dir.join("mif-cli.1"), buffer)?;

    for sub in cmd.get_subcommands() {
        let mut buf = Vec::new();
        Man::new(sub.clone()).render(&mut buf)?;
        fs::write(man_dir.join(format!("mif-cli-{}.1", sub.get_name())), buf)?;
    }

    Ok(())
}
```

`build.rs` returns `Result` and propagates with `?` rather than
`.unwrap()`/`.expect()` — this workspace's `[workspace.lints.clippy]` denies
`unwrap_used` and `expect_used`.

## Step 4 — Build and locate the generated pages

```bash
cargo build -p mif-cli
find target/debug/build/mif-cli-*/out/man -name '*.1'
```

This lists `mif-cli.1`, `mif-cli-validate.1`, and `mif-cli-ontology.1`.

## Step 5 — View a generated page

```bash
man target/debug/build/mif-cli-*/out/man/mif-cli.1
```

The rendered page shows `mif-cli`'s `NAME`, `SYNOPSIS`, and the `validate` and
`ontology` subcommands, generated directly from the `clap` definition in
`crates/mif-cli/src/cli.rs`.
