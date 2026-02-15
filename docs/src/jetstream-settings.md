# JetStream Settings

This page maps `notification_backend.jetstream` settings to runtime behavior.

## Field mapping

| Setting | Default | Applied? | Notes |
|---|---:|---|---|
| `nats_url` | `nats://localhost:4222` | Yes | Used for NATS connection target. |
| `token` | `None` | Yes | Used for token auth (`NATS_TOKEN` env fallback). |
| `timeout_seconds` | `30` | Yes | Applied as NATS `connection_timeout` during connect attempts. |
| `retry_attempts` | `3` | Yes | Controls startup connection retry attempts (`min=1`). |
| `max_messages` | `None` | Yes | Mapped to stream `max_messages`. |
| `max_bytes` | `None` | Yes | Mapped to stream `max_bytes`. |
| `retention_days` | `None` | Yes | Mapped to stream `max_age` (days -> seconds). |
| `storage_type` | `file` | Yes | Parsed at stream creation (`file`/`memory`). |
| `replicas` | `None` | Yes | Mapped to stream `num_replicas`. |
| `retention_policy` | `limits` | Yes | Parsed at stream creation. |
| `discard_policy` | `old` | Yes | Parsed at stream creation. |
| `enable_auto_reconnect` | `true` | Yes | Enables/disables NATS client reconnect behavior. |
| `max_reconnect_attempts` | `5` | Yes | Mapped to NATS `max_reconnects` (`0` means unlimited). |
| `reconnect_delay_ms` | `2000` | Yes | Used for NATS reconnect delay and initial connect retry delay. |

## Important caveats

- Stream config settings only apply on stream creation. Existing streams are not reconciled automatically.
- `storage_type`, `retention_policy`, and `discard_policy` values are validated during stream creation.
- `retry_attempts` applies to startup connect attempts; reconnect behavior after startup uses `enable_auto_reconnect`/`max_reconnect_attempts`.

## Recommended usage

- Use `retry_attempts` to control startup resiliency when NATS is temporarily unavailable.
- Set `max_reconnect_attempts = 0` only if you want unlimited reconnect retries.
- Use stable policy values:
  - `storage_type`: `file` or `memory`
  - `retention_policy`: `limits`, `interest`, `workqueue`
  - `discard_policy`: `old`, `new`
