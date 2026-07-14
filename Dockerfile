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
# Which workspace binary to build into this image: mif-cli or mif-mcp.
# Passed by the CI matrix in release-docker.yml — never guess it here.
ARG BIN
COPY --from=planner /app/recipe.json recipe.json
# Builds only the dependency graph, cached independently of application
# source changes.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p "${BIN}" --bin "${BIN}"

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
