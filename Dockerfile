# Cargo features compiled into the binary. Default includes the ECPDS
# authorization plugin so a deployment that configures `auth.plugins:
# ["ecpds"]` works without rebuilding. Override at build time if a smaller
# image is needed:  --build-arg CARGO_FEATURES="" .
ARG CARGO_FEATURES="ecpds"

###############################
# Stage 1: Prepare Cargo Chef Recipe
###############################
FROM rust:1.90-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
RUN apt-get update && apt-get install -y libssl-dev pkg-config build-essential && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

###############################
# Stage 2: Cache Dependencies
###############################
FROM rust:1.90-slim-bookworm AS cacher
ARG CARGO_FEATURES
RUN cargo install cargo-chef --locked
RUN apt-get update && apt-get install -y libssl-dev pkg-config build-essential curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=chef /app/recipe.json recipe.json
# Both subcrates are path-dependencies and need their full source for
# `cargo chef cook` to resolve, even though only aviso-validators is a
# required dep at default features. aviso-ecpds is required when
# CARGO_FEATURES=ecpds.
COPY --from=chef /app/aviso-validators /app/aviso-validators
COPY --from=chef /app/aviso-ecpds      /app/aviso-ecpds
# Gate `--features` on a non-empty value: passing `--features ""` is
# accepted by some cargo versions but errors on others, and an unquoted
# `${CARGO_FEATURES:+--features $CARGO_FEATURES}` would word-split a
# multi-value setting like `ecpds foo` into broken positional args.
RUN if [ -n "$CARGO_FEATURES" ]; then \
      cargo chef cook --release --features "$CARGO_FEATURES" --recipe-path recipe.json; \
    else \
      cargo chef cook --release --recipe-path recipe.json; \
    fi

###############################
# Stage 3: Build the Project
###############################
FROM rust:1.90-slim-bookworm AS builder
ARG CARGO_FEATURES
RUN apt-get update && apt-get install -y libssl-dev pkg-config build-essential curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
RUN if [ -n "$CARGO_FEATURES" ]; then \
      cargo build --release --features "$CARGO_FEATURES"; \
    else \
      cargo build --release; \
    fi

###############################
# Stage 4: Production Release Image
###############################
# distroless/cc:nonroot pins a specific image variant rather than the
# unlabelled :latest. The included `nonroot` user matches the USER
# directive below.
FROM gcr.io/distroless/cc-debian12:nonroot AS release
ARG VERSION
ARG CARGO_FEATURES
LABEL org.opencontainers.image.version=$VERSION
LABEL org.opencontainers.image.title="aviso-server"
LABEL org.opencontainers.image.source="https://github.com/ecmwf/aviso-server"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL aviso.cargo.features=$CARGO_FEATURES
WORKDIR /app
COPY --from=builder /app/target/release/aviso_server /app/aviso_server
COPY --from=builder /app/configuration/ /app/configuration/
COPY --from=builder /app/src/static /app/static
USER nonroot:nonroot
ENTRYPOINT ["/app/aviso_server"]

###############################
# Stage 5: Debug Image
###############################
FROM debian:bookworm-slim AS debug
ARG VERSION
ARG CARGO_FEATURES
LABEL org.opencontainers.image.version=$VERSION
LABEL org.opencontainers.image.title="aviso-server-debug"
LABEL org.opencontainers.image.source="https://github.com/ecmwf/aviso-server"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL aviso.cargo.features=$CARGO_FEATURES
RUN apt-get update && apt-get install -y ca-certificates bash && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/aviso_server /app/aviso_server
COPY --from=builder /app/configuration/ /app/configuration/
COPY --from=builder /app/src/static /app/static
ENTRYPOINT ["/app/aviso_server"]
