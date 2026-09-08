# Installation

This page covers every way to get Aviso Server running: building from source,
using Docker, or deploying to Kubernetes with Helm.

---

## Prerequisites

### Rust toolchain

Aviso requires **Rust 1.88 or newer**. Each published crate declares this
minimum supported Rust version in its package metadata. CI also builds with the
repository's newer pinned toolchain.

Install or update Rust via [rustup](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Verify your version:

```bash
rustc --version
# rustc 1.88.0 (... ) or newer
```

### System dependencies

Aviso does not require OpenSSL. HTTPS uses rustls with AWS-LC. Building AWS-LC
from source requires a native C/C++ toolchain, CMake, and Perl. The runtime uses
the platform's native certificate store, so Linux installations also need a
current CA certificate bundle.

On Debian/Ubuntu:

```bash
sudo apt-get install -y build-essential cmake perl ca-certificates
```

On Fedora/RHEL:

```bash
sudo dnf install -y gcc gcc-c++ cmake perl ca-certificates
```

On macOS, install the Command Line Tools and CMake. macOS supplies Perl and the
native certificate store:

```bash
xcode-select --install
brew install cmake
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

The server loads `./configuration/config.yaml` by default. See
[Configuration](./configuration.md) for all config loading options.

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

| Target    | Base image             | Size    | Use             |
| --------- | ---------------------- | ------- | --------------- |
| `release` | `distroless/cc`        | minimal | Production      |
| `debug`   | `debian:bookworm-slim` | larger  | Troubleshooting |

---

## Local JetStream (Docker)

For local development with the JetStream backend, use the provided script to
spin up a NATS server with JetStream enabled:

```bash
./scripts/init_nats.sh
```

This script:

- Generates a private config under
  `${XDG_STATE_HOME:-$HOME/.local/state}/aviso/`
- Uses a persistent Docker volume named `${CONTAINER_NAME}-data`
- Starts a `nats:2.14.6-alpine` container on loopback port `4222`
- Waits for the server to be ready and prints a connection summary

**Requires:** Docker and the `nats` CLI. Token generation also uses OpenSSL.

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

The script stores the generated token in the private configuration file and
never prints it. Supply `TOKEN` securely to choose your own token; it may
contain
letters, digits, underscores and hyphens.

Use `NATS_IMAGE` to override the image. `CONFIG_DIR` accepts an absolute or
relative path; `SKIP_DOCKER=1` generates configuration without starting a
server.
Existing containers are never removed automatically. Choose a new
`CONTAINER_NAME`, or stop and remove the old container explicitly to reuse its
data volume. `NATS_VOLUME` selects an existing volume when needed.

For an isolated test broker, give it separate resources and unused ports:

```bash
CONTAINER_NAME=aviso-test DOCKER_NETWORK=aviso-test \
CONFIG_DIR=/tmp/aviso-test NATS_VOLUME=aviso-test-data \
NATS_PORT=14222 NATS_HTTP_PORT=18222 NATS_CLUSTER_PORT=16222 \
./scripts/init_nats.sh
AVISO_RUN_NATS_TESTS=1 NATS_URL=nats://127.0.0.1:14222 cargo test --locked
```

Run opt-in tests only against a disposable broker, never shared streams. An
unreachable broker fails the opted-in tests. The script binds published ports
to `127.0.0.1`; set `NATS_BIND_ADDRESS` explicitly to expose another interface.
Readiness checks use that address. Wildcard binds (`0.0.0.0` or `::`) use the
corresponding loopback address instead. IPv6 addresses may be bracketed or bare.

**After the script completes, configure Aviso to connect:**

Without auth (default):

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
```

With auth, pass the token stored in the private configuration file:

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
    token: "aviso_secure_token_1712345678"
```

Alternatively, set the token as an environment variable (Aviso reads
`NATS_TOKEN` automatically):

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
