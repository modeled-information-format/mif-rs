# Base images are pinned by digest (OpenSSF Scorecard: Pinned-Dependencies).
# The :tag is kept alongside the digest for human readability; Dependabot's
# docker ecosystem keeps the digest fresh. Refresh with:
#   docker buildx imagetools inspect <image> --format '{{.Manifest.Digest}}'
FROM rust:1.97-slim@sha256:14c4fe50ea427dc42381a1a09a9a839c1d2346a2e508cd491bf02c659dbc0ed7 AS chef
RUN cargo install cargo-chef --locked --version 0.1.77
WORKDIR /app

# cargo-chef's whole point: derive a dependency-only "recipe" from the
# workspace's Cargo.toml/Cargo.lock files, so the actual dependency
# compilation (the expensive part, and the part that almost never changes)
# lands in its own Docker layer that only invalidates when a manifest
# changes — not on every source edit. This is the fix for the previous
# single-COPY-then-build design, which recompiled the entire dependency
# tree from scratch on every push regardless of what actually changed.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Builds only the dependency graph, cached independently of application
# source changes. cargo-chef masks local (workspace) crate versions in both
# the manifests and Cargo.lock when generating the recipe, so release-prep
# version bumps do NOT invalidate this layer — only a real dependency
# change does.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
# Build every [[bin]] target in one pass so the shared workspace dependency
# tree (including the candle ML stack via mif-embed) compiles exactly once
# for all images, instead of once per bin. release-docker.yml builds the
# four images sequentially in a single job against this shared stage; each
# per-bin build after the first is a pure BuildKit cache hit up to here.
RUN cargo build --release --locked --bins

# Runtime stage - use Chainguard's glibc-dynamic for minimal attack surface
# while keeping glibc + CA certificates (unlike distroless/static or scratch,
# which have neither — needed for any future HTTPS/TLS use, not just today's
# offline validation logic). Chainguard rebuilds from source continuously
# (Wolfi), unlike Debian-based distroless images which inherit whatever CVEs
# sit unpatched in Debian's stable branch (bookworm carried 14 CVEs Debian
# had explicitly marked <no-dsa> — will never fix — with no way to resolve
# them by bumping the base image; glibc-dynamic currently scans clean).
# Pinned by digest (no :latest) to satisfy Scorecard Pinned-Dependencies and
# Trivy DS-0001; Dependabot's docker ecosystem keeps the digest fresh.
FROM cgr.dev/chainguard/glibc-dynamic@sha256:7ff79e2caef2b8a137ddaf9940fb790e91148482092363760d6661e4591fd54c

# Which workspace binary this image ships: mif-cli, mif-mcp, mif-rh-cli, or
# mif-rh-mcp. Passed per image by release-docker.yml — never guess it here.
# Only this final stage varies by BIN; everything above it is shared.
ARG BIN

# Copy binary from builder. glibc-dynamic has no shell, so BIN can't be
# interpolated in COPY/ENTRYPOINT here — the calling workflow builds one
# image per bin and passes a matching, bin-specific final image tag.
COPY --from=builder /app/target/release/${BIN} /usr/local/bin/app

# Set non-root user
USER nonroot:nonroot

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/app"]

# Run the binary
ENTRYPOINT ["/usr/local/bin/app"]
