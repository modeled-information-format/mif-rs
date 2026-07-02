---
id: how-to-generate-shell-completions-mif-cli
type: procedural
created: '2026-07-02T00:00:00Z'
modified: '2026-07-02T00:00:00Z'
namespace: how-to/cli
title: How to Generate Shell Completions for mif-cli
tags:
  - how-to
  - cli
  - shell-completions
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
  name: Generate Shell Completions for mif-cli
  entity_type: how-to-guide
---

# How to Generate Shell Completions for mif-cli

Add a runtime `completions` subcommand to `mif-cli` that emits a shell
completion script via `clap_complete`, then install it for your shell.

## Prerequisites

- The `mif-rs` workspace checked out, with `cargo build -p mif-cli` working.
- `crates/mif-cli/src/main.rs` defines `Cli` (via `clap::Parser`) with the
  `Command` enum (`Validate`, `Ontology { command: OntologyCommand }`).

## Step 1 — Add clap_complete as a dependency

Edit `crates/mif-cli/Cargo.toml`:

```toml
[dependencies]
clap_complete = "4.6"
```

## Step 2 — Add a Completions variant to the Command enum

In `crates/mif-cli/src/main.rs`, add a variant to `Command`:

```rust
#[derive(Subcommand)]
enum Command {
    /// Validate a MIF document against the canonical schema.
    Validate {
        file: PathBuf,
    },
    /// Ontology-related operations.
    Ontology {
        #[command(subcommand)]
        command: OntologyCommand,
    },
    /// Generate a shell completion script on stdout.
    Completions {
        /// Target shell.
        shell: clap_complete::Shell,
    },
}
```

## Step 3 — Dispatch the new variant

Add `use clap::CommandFactory;` alongside the existing `clap` import, then
handle `Command::Completions` in `run`:

```rust
fn run(command: &Command) -> Result<(), String> {
    match command {
        Command::Validate { file } => validate(file),
        Command::Ontology { command } => match command {
            OntologyCommand::Resolve { id, ontologies_dir } => resolve(id, ontologies_dir),
        },
        Command::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), "mif-cli", &mut std::io::stdout());
            Ok(())
        },
    }
}
```

## Step 4 — Build and generate a completion script

```bash
cargo build -p mif-cli
./target/debug/mif-cli completions bash > mif-cli.bash
```

Substitute `zsh`, `fish`, `powershell`, or `elvish` for another shell.

## Step 5 — Install and load it (bash example)

```bash
mkdir -p ~/.local/share/bash-completion/completions
cp mif-cli.bash ~/.local/share/bash-completion/completions/mif-cli
source ~/.local/share/bash-completion/completions/mif-cli
```

## Step 6 — Verify

```bash
mif-cli <TAB><TAB>
```

The shell lists `validate`, `ontology`, `completions`, and `help` —
completions are wired up for `mif-cli`.
