# Getting Started

## Prerequisites

- Rust toolchain (edition 2024 compatible).
- For JetStream backend: a running NATS server with JetStream enabled.

## Kubernetes / Helm deployment

Helm chart repository:

- <https://github.com/ecmwf/aviso-chart>

## Run with default configuration

From repository root:

```bash
cargo run
```

This loads `configuration/config.yaml`.

## Start a local JetStream environment (Docker)

You can bootstrap a local NATS + JetStream instance with:

```bash
./scripts/init_nats.sh
```

This exposes NATS on `localhost:4222` for local app testing.

Use backend configuration like:

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
```

## Quick endpoint smoke check

Health:

```bash
curl -sS http://127.0.0.1:8000/health
```

Send a notification:

```bash
curl -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "test_polygon",
    "identifier": {
      "date": "20250706",
      "time": "1200",
      "polygon": "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    },
    "payload": { "note": "hello" }
  }'
```

## Run full smoke script

Once the server is running, execute:

```bash
./scripts/smoke_test.sh
```

Useful overrides:

```bash
BASE_URL="http://127.0.0.1:8000" ./scripts/smoke_test.sh
BACKEND="jetstream" ./scripts/smoke_test.sh
TIMEOUT_SECONDS=12 ./scripts/smoke_test.sh
```

## Build and serve docs

```bash
mdbook serve docs --open
```
