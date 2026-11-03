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
# AWS-LC backs rustls and builds bundled native code. OpenSSL is not used.
RUN apt-get update && apt-get install -y build-essential cmake perl ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

###############################
# Stage 2: Cache Dependencies
###############################
FROM rust:1.90-slim-bookworm AS cacher
ARG CARGO_FEATURES
RUN cargo install cargo-chef --locked
RUN apt-get update && apt-get install -y build-essential cmake perl ca-certificates curl && rm -rf /var/lib/apt/lists/*
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
RUN apt-get update && apt-get install -y build-essential cmake perl ca-certificates curl && rm -rf /var/lib/apt/lists/*
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
# Stage 4: CA certificate bundle
###############################
# The bundle in gcr.io/distroless/cc-debian12 can lag behind Debian. The
# server verifies outgoing TLS against this bundle, so the images take it
# from here instead. The build fails if the bundle still trusts a root
# behind the compromised eMudhra intermediates (CVE-2026-111943):
# ca-certificates 20250419~deb12u2 removes emSign ECC Root CA - G3 and AAA
# Certificate Services; emSign Root CA - C1 and emSign ECC Root CA - C3,
# removed by Mozilla since, are deselected here until Debian follows.
# The RUN step uses VERSION so that each release rebuilds this stage
# instead of reusing a cached, older bundle.
FROM debian:bookworm-slim AS certs
ARG VERSION
RUN echo "CA bundle for aviso-server ${VERSION}" \
 && apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && dpkg --compare-versions "$(dpkg-query -W -f '${Version}' ca-certificates)" ge 20250419~deb12u2 \
 && sed -i -e 's|^mozilla/emSign_Root_CA_-_C1.crt$|!&|' \
           -e 's|^mozilla/emSign_ECC_Root_CA_-_C3.crt$|!&|' /etc/ca-certificates.conf \
 && update-ca-certificates --fresh >/dev/null \
 && mkdir /tmp/bundle && cd /tmp/bundle \
 && awk '/BEGIN CERTIFICATE/ {n++} n {print > ("cert" n ".pem")}' /etc/ssl/certs/ca-certificates.crt \
 && count=$(find . -name 'cert*.pem' | wc -l) \
 && if [ "$count" -lt 100 ]; then echo "unexpected CA bundle: $count certificates" >&2; exit 1; fi \
 && for c in cert*.pem; do openssl x509 -noout -subject -nameopt RFC2253 -in "$c"; done > subjects \
 && if [ "$(wc -l < subjects)" -ne "$count" ]; then echo "unreadable certificate in the CA bundle" >&2; exit 1; fi \
 && if grep -E '(=|,)CN=(emSign ECC Root CA - G3|emSign Root CA - C1|emSign ECC Root CA - C3|AAA Certificate Services)(,|$)' \
         subjects >&2; then \
      echo "distrusted root still in the CA bundle" >&2; exit 1; \
    elif [ $? -ne 1 ]; then echo "could not check the CA bundle" >&2; exit 1; fi \
 && cd / && rm -rf /tmp/bundle

###############################
# Stage 5: Production Release Image
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
LABEL aviso.cargo.features="$CARGO_FEATURES"
WORKDIR /app
COPY --from=builder /app/target/release/aviso_server /app/aviso_server
COPY --from=builder /app/configuration/ /app/configuration/
COPY --from=builder /app/src/static /app/static
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
USER nonroot:nonroot
ENTRYPOINT ["/app/aviso_server"]

###############################
# Stage 6: Debug Image
###############################
# Built on the certs stage, so it shares its CA store and selection.
FROM certs AS debug
ARG VERSION
ARG CARGO_FEATURES
LABEL org.opencontainers.image.version=$VERSION
LABEL org.opencontainers.image.title="aviso-server-debug"
LABEL org.opencontainers.image.source="https://github.com/ecmwf/aviso-server"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL aviso.cargo.features="$CARGO_FEATURES"
RUN apt-get update && apt-get install -y bash && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/aviso_server /app/aviso_server
COPY --from=builder /app/configuration/ /app/configuration/
COPY --from=builder /app/src/static /app/static
ENTRYPOINT ["/app/aviso_server"]
