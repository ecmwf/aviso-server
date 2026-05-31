# ────────────────────────────────────────────────────────────────
# aviso-server CI image  ->  eccr.ecmwf.int/aviso/server-ci
#
# Pre-bakes the Rust toolchain, sccache, and the mdbook doc tools so CI
# jobs only compile the workspace, never the toolchain. Build and push:
#
#   docker build -t eccr.ecmwf.int/aviso/server-ci:test .github/ci/
# ────────────────────────────────────────────────────────────────

# Bump these to refresh the baked tools, then bump .github/ci/VERSION.
ARG RUST_VERSION=1.95.0
ARG SCCACHE_VERSION=0.15.0
ARG MDBOOK_VERSION=0.5.3
ARG MDBOOK_MERMAID_VERSION=0.17.0

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# gcc/g++/make/cmake/perl/pkg-config: aws-lc-sys, ring and zstd-sys
# (pulled in via the rustls/aws-lc-rs stack and async-nats) build C/asm
# from source, drive CMake, and probe with pkg-config. git: checkout +
# the Cargo.lock-diff gate.
# ca-certificates: TLS to crates.io and the sccache S3 bucket.
RUN set -eux \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      gcc g++ make cmake perl pkg-config libc6-dev git curl ca-certificates \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* \
           /usr/share/doc /usr/share/man /usr/share/info

# Rust toolchain pinned to rust-toolchain.toml. rustup default avoids a
# re-resolve when cargo reads rust-toolchain.toml from a checkout.
ARG RUST_VERSION
RUN set -eux \
 && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain "${RUST_VERSION}" \
                 --profile minimal -c clippy -c rustfmt \
 && rustup default "${RUST_VERSION}" \
 && rm -rf "${RUSTUP_HOME}"/toolchains/*/share/doc \
           "${RUSTUP_HOME}"/toolchains/*/share/man /tmp/*

# Prebuilt static (musl) binaries: smaller than compiling from source and
# the image build stays in seconds. sccache caches rustc; mdbook +
# mdbook-mermaid render the docs (docs-sites.yaml builds in this image so
# the tool versions live here, not duplicated in the workflow).
ARG SCCACHE_VERSION
ARG MDBOOK_VERSION
ARG MDBOOK_MERMAID_VERSION
RUN set -eux \
 && cd /usr/local/bin \
 && curl -fsSL "https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz --strip-components=1 "sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache" \
 && curl -fsSL "https://github.com/rust-lang/mdBook/releases/download/v${MDBOOK_VERSION}/mdbook-v${MDBOOK_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz mdbook \
 && curl -fsSL "https://github.com/badboy/mdbook-mermaid/releases/download/v${MDBOOK_MERMAID_VERSION}/mdbook-mermaid-v${MDBOOK_MERMAID_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz mdbook-mermaid

RUN set -eux \
 && rustc --version && cargo --version \
 && cargo clippy --version && rustfmt --version \
 && cmake --version && sccache --version \
 && mdbook --version && mdbook-mermaid --version

WORKDIR /workspace
