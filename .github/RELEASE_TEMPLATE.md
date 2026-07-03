## What's Changed

<!-- Automatically filled by git-cliff -->

## Installation

### Binary Releases

Download pre-built binaries for your platform:

**mif-cli**

- **Linux (x86_64)**: `mif-cli-VERSION-linux-amd64`
- **Linux (ARM64)**: `mif-cli-VERSION-linux-arm64`
- **macOS (x86_64)**: `mif-cli-VERSION-macos-amd64`
- **macOS (ARM64)**: `mif-cli-VERSION-macos-arm64`
- **Windows (x86_64)**: `mif-cli-VERSION-windows-amd64.exe`

**mif-mcp**

- **Linux (x86_64)**: `mif-mcp-VERSION-linux-amd64`
- **Linux (ARM64)**: `mif-mcp-VERSION-linux-arm64`
- **macOS (x86_64)**: `mif-mcp-VERSION-macos-amd64`
- **macOS (ARM64)**: `mif-mcp-VERSION-macos-arm64`
- **Windows (x86_64)**: `mif-mcp-VERSION-windows-amd64.exe`

### Cargo

```bash
cargo install mif-cli@VERSION
cargo install mif-mcp@VERSION
```

### crates.io Libraries

All 9 workspace crates are published independently at the same version.
These are the 7 library crates (`mif-cli` and `mif-mcp` are binaries —
install those via `cargo install` above):

```bash
cargo add mif-core@VERSION
cargo add mif-problem@VERSION
cargo add mif-schema@VERSION
cargo add mif-frontmatter@VERSION
cargo add mif-ontology@VERSION
cargo add mif-embed@VERSION
cargo add mif-store@VERSION
```

### Docker

```bash
docker pull ghcr.io/modeled-information-format/mif-rs:VERSION
```

## Verification

### Binary Checksums

<!-- Add checksums here -->

### Docker Image

```bash
docker pull ghcr.io/modeled-information-format/mif-rs:VERSION
docker run --rm ghcr.io/modeled-information-format/mif-rs:VERSION --version
```

## Full Changelog

See [CHANGELOG.md](https://github.com/modeled-information-format/mif-rs/blob/main/CHANGELOG.md) for complete details.
