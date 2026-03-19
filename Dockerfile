# syntax=docker/dockerfile:1.9
# Multi-stage build for Rust TurboVault Server

# Stage 1: Builder
FROM rust:latest as builder

WORKDIR /build

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build server in release mode with HTTP transport
RUN cargo build --release --package turbovault --features http

# Stage 2: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /build/target/release/turbovault /usr/local/bin/

# Create non-root user
RUN useradd -m -u 1000 obsidian

# Create vault directory
RUN mkdir -p /var/obsidian-vault && chown obsidian:obsidian /var/obsidian-vault

# Switch to non-root user
USER obsidian

# Set working directory
WORKDIR /var/obsidian-vault

# Environment variables
ENV RUST_LOG=info
ENV OBSIDIAN_VAULT_PATH=/var/obsidian-vault

# Expose HTTP port
EXPOSE 3000

# Health check via HTTP
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:3000/v1/health || exit 1

# Run server with HTTP transport, bind to all interfaces
ENTRYPOINT ["/usr/local/bin/turbovault", "--profile", "production", "--init", "--transport", "http", "--bind", "0.0.0.0"]
