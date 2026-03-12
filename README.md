<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
    <img alt="Aviso Logo" src="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
  </picture>
</div>

<p align="center">
  <a href="https://github.com/ecmwf/codex/raw/refs/heads/main/ESEE">
    <img src="https://github.com/ecmwf/codex/raw/refs/heads/main/ESEE/foundation_badge.svg" alt="Foundation Badge">
  </a>
  <a href="https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity">
    <img src="https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity/emerging_badge.svg" alt="Maturity Badge">
  </a>
</p>

> [!IMPORTANT]  
> This software is **Emerging** and subject to ECMWF's guidelines on [Software Maturity](https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity).

## Overview

Aviso Server is a notification service for data-driven workflows.

It helps you answer questions like:
- "What just arrived?"
- "Give me updates for this exact subset of data."
- "Replay everything I missed since yesterday."

Producers publish notifications once, and consumers can either follow live updates or replay history using the same filter model. Aviso keeps this predictable by validating identifiers against schema rules and streaming notifications in a consistent event format.
For regional use cases, Aviso also supports spatial filtering so clients can subscribe to notifications relevant to a specific area or point.

## Key Features

- Publish notifications through a simple HTTP API
- Watch live updates over SSE with connection and replay controls
- Replay historical notifications by sequence or timestamp
- Filter by exact identifier values or constraints (for supported field types)
- Use spatial filters for polygon/point use cases
- Run with either in-memory storage (local/dev) or JetStream (durable environments)

## Quick Start

### Run locally (in-memory backend)

```bash
cargo run
```

### Run tests

```bash
cargo test --workspace
```

JetStream integration tests are opt-in:

```bash
AVISO_RUN_NATS_TESTS=1 cargo test --workspace
```

## Build Docs Locally

```bash
cargo install mdbook
mdbook build docs
mdbook serve docs --open
```

## Authentication (Optional)

Aviso Server supports authentication via [auth-o-tron](https://github.com/ecmwf/auth-o-tron) as an external authentication service.

### Quick Start with Auth

1. Start auth-o-tron using Docker:
```bash
./scripts/auth-o-tron-docker.sh
```

By default this uses `scripts/example_auth_config.yaml`.
Use a custom config if needed:

```bash
AUTH_O_TRON_CONFIG_FILE=/path/to/auth-config.yaml ./scripts/auth-o-tron-docker.sh
```

2. Configure auth in `configuration/config.yaml`:
```yaml
auth:
  enabled: true
  auth_o_tron_url: "http://localhost:8080"
  jwt_secret: "your-shared-secret" # must match auth-o-tron jwt.secret
  admin_roles: ["admin", "superuser"]
  timeout_ms: 5000
```

3. Run aviso-server with auth enabled

### Per-Stream Authentication

You can configure authentication requirements per stream in your notification schema:

```yaml
notification_schema:
  # Authenticated stream - any valid user
  public_stream:
    # ... other config
    auth:
      required: true  # Any authenticated user allowed

  # Restricted stream - specific roles only
  admin_stream:
    # ... other config
    auth:
      required: true
      allowed_roles: ["admin", "operator"]
```

### Admin Endpoints

Admin endpoints (`/api/v1/admin/*`) require users to have one of the configured `admin_roles`.

### Disabling Auth

To disable authentication completely:
```yaml
auth:
  enabled: false
```

Or omit the `auth` section entirely from your configuration.
