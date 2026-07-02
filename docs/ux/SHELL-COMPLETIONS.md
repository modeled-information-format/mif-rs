---
diataxis_type: how-to
---
# Shell Completions

## Overview

Generate shell completions for enhanced command-line UX using [clap_complete](https://docs.rs/clap_complete).

## Setup

### Add Dependencies

```toml
[dependencies]
clap = { version = "4.5", features = ["derive"] }
clap_complete = "4.5"
```

### Implement Completions

**crates/cli.rs:**

```rust
use clap::{Parser, CommandFactory};
use clap_complete::{generate, Shell};
use std::io;

#[derive(Parser, Debug)]
#[command(name = "mif-rs")]
#[command(about = "Modern Rust project template")]
#[command(version)]
pub struct Cli {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Generate shell completions
    #[arg(long, value_name = "SHELL")]
    pub completions: Option<Shell>,
}

impl Cli {
    pub fn generate_completions(shell: Shell) {
        let mut cmd = Self::command();
        generate(shell, &mut cmd, "mif-rs", &mut io::stdout());
    }
}
```

**crates/main.rs:**

```rust
use clap::Parser;
use cli::Cli;

fn main() {
    let cli = Cli::parse();

    // Handle completion generation
    if let Some(shell) = cli.completions {
        Cli::generate_completions(shell);
        return;
    }

    // Normal application logic
    run(cli);
}
```

## Installation

### Bash

```bash
# Generate completions
mif-rs --completions bash > ~/.local/share/bash-completion/completions/mif-rs

# Or system-wide
sudo mif-rs --completions bash > /etc/bash_completion.d/mif-rs

# Reload
source ~/.bashrc
```

**Test:**
```bash
mif-rs --<TAB>
# Shows: --config --verbose --help --version --completions
```

### Zsh

```bash
# Generate completions
mif-rs --completions zsh > ~/.zsh/completions/_mif-rs

# Add to .zshrc if not already
echo 'fpath=(~/.zsh/completions $fpath)' >> ~/.zshrc
echo 'autoload -Uz compinit && compinit' >> ~/.zshrc

# Reload
source ~/.zshrc
```

**Test:**
```bash
mif-rs --<TAB>
# Shows completion menu with descriptions
```

### Fish

```bash
# Generate completions
mif-rs --completions fish > ~/.config/fish/completions/mif-rs.fish

# Reload (automatic in most cases)
fish -c 'fish_update_completions'
```

**Test:**
```bash
mif-rs --<TAB>
# Shows completions with descriptions
```

### PowerShell

```powershell
# Generate completions
mif-rs --completions powershell | Out-File -FilePath $PROFILE\..\mif-rs.ps1

# Add to profile
Add-Content $PROFILE '. "$PSScriptRoot\mif-rs.ps1"'

# Reload
. $PROFILE
```

**Test:**
```powershell
mif-rs --<TAB>
# Shows completion suggestions
```

### Elvish

```bash
# Generate completions
mif-rs --completions elvish > ~/.elvish/lib/mif-rs.elv

# Add to rc.elv
echo 'use mif-rs' >> ~/.elvish/rc.elv
```

## Package Integration

### Homebrew

**Formula includes completions:**

```ruby
def install
  system "cargo", "install", *std_cargo_args

  # Generate completions
  bash_completion.install "completions/mif-rs.bash"
  zsh_completion.install "completions/_mif-rs"
  fish_completion.install "completions/mif-rs.fish"
end
```

**Or generate during install:**

```ruby
def install
  system "cargo", "install", *std_cargo_args

  # Generate at install time
  generate_completions_from_executable(bin/"mif-rs", "--completions")
end
```

### Debian Package

**Cargo.toml:**

```toml
[package.metadata.deb]
assets = [
    ["target/release/mif-rs", "usr/bin/", "755"],
    ["completions/mif-rs.bash", "usr/share/bash-completion/completions/", "644"],
    ["completions/_mif-rs", "usr/share/zsh/vendor-completions/", "644"],
    ["completions/mif-rs.fish", "usr/share/fish/vendor_completions.d/", "644"],
]
```

### Build Script

**build.rs:**

```rust
use clap::CommandFactory;
use clap_complete::{generate_to, Shell};
use std::env;
use std::path::PathBuf;

include!("crates/cli.rs");

fn main() {
    let outdir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut cmd = Cli::command();

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
        generate_to(shell, &mut cmd, "mif-rs", &outdir).unwrap();
    }

    println!("cargo:rerun-if-changed=crates/cli.rs");
}
```

## Advanced Features

### Subcommands

```rust
#[derive(Parser)]
enum Commands {
    /// Initialize a new project
    Init {
        /// Project name
        name: String,
    },
    /// Build the project
    Build {
        /// Release mode
        #[arg(short, long)]
        release: bool,
    },
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
```

**Completions automatically include subcommands:**
```bash
mif-rs <TAB>
# Shows: init, build, help
```

### Dynamic Completions

```rust
use clap::ValueHint;

#[derive(Parser)]
struct Cli {
    /// Input file
    #[arg(value_hint = ValueHint::FilePath)]
    input: String,

    /// Output directory
    #[arg(value_hint = ValueHint::DirPath)]
    output: String,

    /// Command to run
    #[arg(value_hint = ValueHint::CommandName)]
    command: String,
}
```

**Hints enable:**
- File/directory path completion
- Command name completion
- URL completion
- Username completion

### Custom Completions

```rust
use clap::builder::PossibleValue;

#[derive(Parser)]
struct Cli {
    /// Log level
    #[arg(value_parser = ["debug", "info", "warn", "error"])]
    level: String,

    /// Or with descriptions
    #[arg(value_parser = [
        PossibleValue::new("debug").help("Detailed debug information"),
        PossibleValue::new("info").help("General information"),
        PossibleValue::new("warn").help("Warning messages"),
        PossibleValue::new("error").help("Error messages only"),
    ])]
    level_detailed: String,
}
```

## Testing Completions

### Manual Testing

```bash
# Bash
complete -p mif-rs

# Zsh
which _mif-rs

# Fish
complete -C mif-rs
```

### Automated Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap_complete::generate;
    use std::io;

    #[test]
    fn verify_completions() {
        let mut cmd = Cli::command();

        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            let mut buf = Vec::new();
            generate(shell, &mut cmd, "mif-rs", &mut buf);
            assert!(!buf.is_empty(), "Generated empty completions for {:?}", shell);
        }
    }
}
```

## Troubleshooting

### Completions Not Working

**Bash:**
```bash
# Check if bash-completion is installed
dpkg -l bash-completion  # Debian/Ubuntu
rpm -q bash-completion   # Fedora/RHEL

# Verify completion file
cat ~/.local/share/bash-completion/completions/mif-rs
```

**Zsh:**
```bash
# Check fpath
echo $fpath

# Verify compinit loaded
which compinit

# Rebuild completion cache
rm -f ~/.zcompdump && compinit
```

**Fish:**
```bash
# Check completions directory
ls ~/.config/fish/completions/

# Reload completions
fish_update_completions
```

### Wrong Completions Shown

```bash
# Clear shell completion cache

# Bash
hash -r

# Zsh
rehash

# Fish
commandline -f repaint
```

## Best Practices

1. **Generate at install time** - Use build.rs or post-install scripts
2. **Include in packages** - Add to .deb, .rpm, Homebrew formula
3. **Document installation** - Provide clear user instructions
4. **Test all shells** - Verify bash, zsh, fish work correctly
5. **Use value hints** - Improve path/file completion UX
6. **Provide subcommand help** - Add descriptions to all commands

## Links

- [clap Documentation](https://docs.rs/clap/)
- [clap_complete Documentation](https://docs.rs/clap_complete/)
- [Bash Completion Guide](https://github.com/scop/bash-completion)
- [Zsh Completion Guide](https://github.com/zsh-users/zsh-completions)
- [Fish Completion Tutorial](https://fishshell.com/docs/current/completions.html)
