<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
    <img alt="Aviso Logo" src="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
  </picture>
</div>

<p align="center">
  <a href="https://crates.io/crates/aviso-server">
    <img src="https://img.shields.io/crates/v/aviso-server.svg" alt="Crates.io Badge">
  </a>
  <a href="https://sites.ecmwf.int/docs/aviso-server/main/">
    <img src="https://img.shields.io/badge/docs-online-blue" alt="Docs Badge">
  </a>
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
- Optional ECPDS destination authorization plugin (Cargo feature `ecpds`)
- Operational endpoints: `/health` for liveness/readiness, `/metrics` for Prometheus scrapes
- Per-response `X-Request-ID` header (and the same UUID in error bodies and SSE first events) for log/trace correlation

## Quick Start

### Run locally (in-memory backend)

```bash
cargo run
```

### Run tests

`aviso-server` is the top-level crate; `aviso-validators/` and `aviso-ecpds/` are path-dependency subcrates with their own test suites:

```bash
cargo test                                                   # aviso-server (default)
cargo test --features ecpds                                  # aviso-server with ECPDS plugin
cargo test --manifest-path aviso-validators/Cargo.toml       # aviso-validators
cargo test --manifest-path aviso-ecpds/Cargo.toml            # aviso-ecpds
```

JetStream integration tests are opt-in (require a local NATS server):

```bash
AVISO_RUN_NATS_TESTS=1 cargo test
```

## Documentation

The full documentation is hosted at <https://sites.ecmwf.int/docs/aviso-server/main/>.

To build locally:

```bash
cargo install mdbook
mdbook serve docs --open
```

Start with [Getting Started](./docs/src/getting-started.md), [Configuration Reference](./docs/src/configuration-reference.md), and the [Practical Examples](./docs/src/practical-examples/overview.md).

## Operating Notes

A few operator-facing details the deeper docs cover in detail:

- **Runtime log filter override.** Set `RUST_LOG` (full [`EnvFilter` directive syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives)) to bump a single module to debug without redeploy, for example `RUST_LOG=info,aviso_server::auth=debug`. When unset, `logging.level` from config plus a small set of muted framework targets is used. See [Configuration Reference](./docs/src/configuration-reference.md#logging).
- **Config file override.** `AVISOSERVER_CONFIG_FILE=/path/to/file.yaml` loads only that file, skipping the standard search path. Environment variables prefixed with `AVISOSERVER_` (nested via `__`) still override individual values. See [Configuration](./docs/src/configuration.md#loading-precedence).
- **Request correlation.** Every response carries `X-Request-ID`; every aviso error body and every SSE stream's first event repeats the same UUID. Quote it when reporting issues. See [API Errors > How to report a problem](./docs/src/api-errors.md#how-to-report-a-problem).
- **Health and metrics.** `GET /health` returns `200 OK` for probes. `/metrics` runs on a separate port (configured under `metrics:`) and exposes Prometheus text-format metrics: per-route HTTP request counts and latency histograms, notifications, SSE connections/delivered events/connection durations, auth outcomes, build info, and (when built with `--features ecpds`) ECPDS cache and access decisions. See the [metrics table](./docs/src/configuration-reference.md#metrics) for the full inventory.

## Authentication (Optional)

Aviso Server supports authentication via [auth-o-tron](https://github.com/ecmwf/auth-o-tron) as an external authentication service. Two modes are available: `direct` (Aviso forwards credentials to auth-o-tron) and `trusted_proxy` (an upstream proxy mints a JWT that Aviso validates locally).

### Quick Start with Auth

1. Start auth-o-tron using Docker:

   ```bash
   ./scripts/auth-o-tron-docker.sh
   ```

   By default this uses `scripts/example_auth_config.yaml`. Override with `AUTH_O_TRON_CONFIG_FILE=/path/to/auth-config.yaml`.

2. Configure auth in `configuration/config.yaml`:

   ```yaml
   auth:
     enabled: true
     mode: direct                        # or "trusted_proxy"
     auth_o_tron_url: "http://localhost:8080"
     jwt_secret: "your-shared-secret"    # must match auth-o-tron jwt.secret
     admin_roles:
       your-realm: ["admin", "superuser"]
     timeout_ms: 5000
   ```

3. Run aviso-server:

   ```bash
   cargo run
   ```

### Per-Stream Authentication

Configure authentication requirements per stream in your notification schema:

```yaml
notification_schema:
  # Public stream: any client can read or write (no auth block).
  public_stream:
    # ... other config

  # Authenticated stream: any valid user can read; only admins can write.
  internal_stream:
    # ... other config
    auth:
      required: true

  # Separate read/write roles.
  restricted_stream:
    # ... other config
    auth:
      required: true
      read_roles:
        your-realm: ["*"]
      write_roles:
        your-realm: ["admin", "operator"]
```

See [Authentication](./docs/src/authentication.md) for the full ruleset and the [Read vs Write defaults table](./docs/src/authentication.md#read-vs-write-access-defaults).

### Admin Endpoints

Admin endpoints (`/api/v1/admin/*`) require users to have one of the configured `admin_roles`.

### Disabling Auth

To disable authentication completely:

```yaml
auth:
  enabled: false
```

Or omit the `auth` section entirely from your configuration.

### ECPDS Destination Authorization (Optional)

When built with `--features ecpds`, aviso supports an optional authorization plugin that checks per-destination access against the ECMWF Production Data Service before allowing `watch` or `replay` on streams that declare `auth.plugins: ["ecpds"]`. The plugin never runs on `notify`. See [ECPDS Destination Authorization](./docs/src/authentication.md#ecpds-destination-authorization) for setup and the [ECPDS Plugin Runbook](./docs/src/ecpds-runbook.md) for on-call triage.
