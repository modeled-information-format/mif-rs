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
- `crates/mif-cli/src/main.rs` defines `Cli` (via `clap::Parser`, with a
  global `--format pretty|json` flag) and the `Command` enum (`Validate`,
  `Ontology { command: OntologyCommand }`, `Ingest`, `Search`, `FindSimilar`,
  `CorpusStats`).

## Step 1 — Add clap_complete as a dependency

Edit `crates/mif-cli/Cargo.toml`:

```toml
[dependencies]
clap_complete = "4.6"
```

## Step 2 — Add a Completions variant to the Command enum

`crates/mif-cli/src/main.rs` already defines `Cli` with a global `--format`
flag and a `Command` enum with six real subcommands:

```rust
#[derive(Parser)]
#[command(
    name = "mif-cli",
    version,
    about = "CLI for the MIF (Modeled Information Format) ecosystem"
)]
struct Cli {
    /// Error rendering format. Defaults to `pretty` on a terminal and `json`
    /// otherwise.
    #[arg(long, global = true, value_parser = ["pretty", "json"])]
    format: Option<String>,
    #[command(subcommand)]
    command: Command,
}

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
    /// Lint, validate, prove a lossless round trip, compute an embedding,
    /// and store the embedding vector for one MIF document.
    Ingest {
        file: PathBuf,
        #[arg(long)]
        db_path: Option<PathBuf>,
    },
    /// Free-text semantic search over previously ingested documents.
    Search {
        query: String,
        #[arg(long)]
        db_path: Option<PathBuf>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Find previously ingested documents similar to an already-ingested one.
    FindSimilar {
        id: String,
        #[arg(long)]
        db_path: Option<PathBuf>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Summary statistics over the vector store.
    CorpusStats {
        #[arg(long)]
        db_path: Option<PathBuf>,
    },
}
```

Add a seventh variant for the new subcommand:

```rust
    /// Generate a shell completion script on stdout.
    Completions {
        /// Target shell.
        shell: clap_complete::Shell,
    },
```

## Step 3 — Dispatch the new variant

Add `use clap::CommandFactory;` alongside the existing `clap` import, then
handle `Command::Completions` in `run` alongside the six existing arms:

```rust
fn run(command: &Command) -> Result<(), String> {
    match command {
        Command::Validate { file } => validate(file),
        Command::Ontology { command } => match command {
            OntologyCommand::Resolve { id, ontologies_dir } => resolve(id, ontologies_dir),
        },
        Command::Ingest { file, db_path } => ingest(file, db_path.as_deref()),
        Command::Search { query, db_path, limit } => search(query, db_path.as_deref(), *limit),
        Command::FindSimilar { id, db_path, limit } => find_similar(id, db_path.as_deref(), *limit),
        Command::CorpusStats { db_path } => corpus_stats(db_path.as_deref()),
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

The shell lists `validate`, `ontology`, `ingest`, `search`, `find-similar`,
`corpus-stats`, `completions`, and `help` — completions are wired up for
`mif-cli`.
