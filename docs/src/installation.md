# Installation

This page covers every way to get Aviso Server running: building from source,
using Docker, or deploying to Kubernetes with Helm.

---

## Prerequisites

### Rust toolchain

Aviso is written in Rust and requires **edition 2024**, which means **Rust 1.85 or newer**.
The CI always builds against the latest stable toolchain.

Install or update Rust via [rustup](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Verify your version:

```bash
rustc --version
# rustc 1.85.0 (... ) or newer
```

### System dependencies

Aviso links against OpenSSL. On Debian/Ubuntu:

```bash
sudo apt-get install -y libssl-dev pkg-config build-essential
```

On Fedora/RHEL:

```bash
sudo dnf install -y openssl-devel pkg-config gcc
```

On macOS (with Homebrew):

```bash
brew install openssl pkg-config
```

---

## Build from Source

Clone the repository:

```bash
git clone https://github.com/ecmwf/aviso-server.git
cd aviso-server
```

### Development build

Fast to compile, includes debug symbols:

```bash
cargo build
```

Binary location: `target/debug/aviso_server`

### Release build

Optimized for production use:

```bash
cargo build --release
```

Binary location: `target/release/aviso_server`

### Run directly

```bash
cargo run                          # development
cargo run --release                # release
./target/release/aviso_server      # pre-built binary
```

The server loads `./configuration/config.yaml` by default.
See [Configuration](./configuration.md) for all config loading options.

---

## Docker

The repository includes a multi-stage `Dockerfile` that produces a minimal
[distroless](https://github.com/GoogleContainerTools/distroless) image.

### Build the image

```bash
# Production image (distroless, minimal attack surface)
docker build --target release -t aviso-server:local .

# Debug image (Debian slim, includes bash for troubleshooting)
docker build --target debug -t aviso-server:debug .
```

### Run with Docker

Mount your config file and expose the port:

```bash
docker run --rm \
  -p 8000:8000 \
  -v $(pwd)/configuration/config.yaml:/app/configuration/config.yaml:ro \
  aviso-server:local
```

Or override settings via environment variables (no config mount needed):

```bash
docker run --rm \
  -p 8000:8000 \
  -e AVISOSERVER_APPLICATION__HOST=0.0.0.0 \
  -e AVISOSERVER_APPLICATION__PORT=8000 \
  -e AVISOSERVER_NOTIFICATION_BACKEND__KIND=in_memory \
  aviso-server:local
```

### Build targets summary

| Target | Base image | Size | Use |
|---|---|---|---|
| `release` | `distroless/cc` | minimal | Production |
| `debug` | `debian:bookworm-slim` | larger | Troubleshooting |

---

## Local JetStream (Docker)

For local development with the JetStream backend, use the provided script to
spin up a NATS server with JetStream enabled:

```bash
./scripts/init_nats.sh
```

This script:

- Generates a NATS config file in `./nats_config/`
- Creates a Docker volume for persistent JetStream storage
- Starts a `nats:2-alpine` container on `localhost:4222`
- Waits for the server to be ready and prints a connection summary

**Requires:** Docker

Optional environment variables:

```bash
NATS_PORT=4222            # NATS client port (default: 4222)
ENABLE_AUTH=true          # Enable token auth (default: false)
MAX_MEMORY=5GB            # JetStream memory limit (default: 5GB)
MAX_STORAGE=10GB          # JetStream file storage limit (default: 10GB)
```

Example with auth enabled:

```bash
ENABLE_AUTH=true ./scripts/init_nats.sh
```

The script prints the generated token at the end of its output, for example:

```
Authentication enabled with token: aviso_secure_token_1712345678
```

**After the script completes, configure Aviso to connect:**

Without auth (default):

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
```

With auth — pass the token printed by the script:

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
    token: "aviso_secure_token_1712345678"
```

Alternatively, set the token as an environment variable (Aviso reads `NATS_TOKEN` automatically):

```bash
export NATS_TOKEN=aviso_secure_token_1712345678
cargo run
```

---

## Kubernetes / Helm

For production Kubernetes deployments, use the official Helm chart:

- **Chart repository:** <https://github.com/ecmwf/aviso-chart>

The chart handles:

- Deployment with configurable replicas
- ConfigMap-based configuration mounting
- Service and Ingress setup
- JetStream connection settings via values

---

## Build Documentation

Aviso docs are built with [mdBook](https://rust-lang.github.io/mdBook/).

Install mdBook and the mermaid preprocessor:

```bash
cargo install mdbook
cargo install mdbook-mermaid
```

Serve docs locally with live reload:

```bash
mdbook serve docs --open
```

Build static output to `docs/book/`:

```bash
mdbook build docs
```

---

## Run Tests

```bash
# Unit and integration tests (in-memory backend)
cargo test --workspace

# Include JetStream integration tests (requires running NATS)
AVISO_RUN_NATS_TESTS=1 cargo test --workspace

# Tests must run single-threaded (shared port binding)
cargo test -- --test-threads=1
```
