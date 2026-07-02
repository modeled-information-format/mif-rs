# Build stage
# Base images are pinned by digest (OpenSSF Scorecard: Pinned-Dependencies).
# The :tag is kept alongside the digest for human readability; Dependabot's
# docker ecosystem keeps the digest fresh. Refresh with:
#   docker buildx imagetools inspect <image> --format '{{.Manifest.Digest}}'
FROM rust:1.96-slim@sha256:c37af730be4fd8104cbf9aedbd6ab259e51ca2d5437817a0f8680edf66ac6c28 AS builder

# Which workspace binary to build into this image: mif-cli or mif-mcp.
# Passed by the CI matrix in release-docker.yml — never guess it here.
ARG BIN

WORKDIR /app

# Install build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy the whole workspace. A per-crate dependency-precaching layer (as the
# single-package template used) needs a valid dummy source file for every
# member and was judged not worth the risk of a subtle multi-crate breakage
# for this bootstrap — one copy, one build, simpler and easier to get right.
COPY . .

# Build the selected binary in release mode.
RUN cargo build --release --locked -p "${BIN}" --bin "${BIN}"

# Runtime stage - use distroless for minimal attack surface.
# Pinned by digest (no :latest) to satisfy Scorecard Pinned-Dependencies and
# Trivy DS-0001; Dependabot's docker ecosystem keeps the digest fresh.
FROM gcr.io/distroless/cc-debian12@sha256:d703b626ba455c4e6c6fbe5f36e6f427c85d51445598d564652a2f334179f96e

ARG BIN

# Copy binary from builder. distroless has no shell, so BIN can't be
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
