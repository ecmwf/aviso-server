# JetStream Settings

This page maps `notification_backend.jetstream` settings to runtime behavior.

## Field mapping

| Setting | Default | Applied? | Notes |
|---|---:|---|---|
| `nats_url` | `nats://localhost:4222` | Yes | Used for NATS connection target. |
| `token` | `None` | Yes | Used for token auth (`NATS_TOKEN` env fallback). |
| `timeout_seconds` | `30` | No | Present in config struct; not consumed by backend operations. |
| `retry_attempts` | `3` | No | Present in config struct; not consumed by backend operations. |
| `max_messages` | `None` | Yes | Mapped to stream `max_messages`. |
| `max_bytes` | `None` | Yes | Mapped to stream `max_bytes`. |
| `retention_days` | `None` | Yes | Mapped to stream `max_age` (days -> seconds). |
| `storage_type` | `file` | Yes | Parsed at stream creation (`file`/`memory`). |
| `replicas` | `None` | Yes | Mapped to stream `num_replicas`. |
| `retention_policy` | `limits` | Yes | Parsed at stream creation. |
| `discard_policy` | `old` | Yes | Parsed at stream creation. |
| `enable_auto_reconnect` | `true` | Partially | Only used for subscription creation retry loop. |
| `max_reconnect_attempts` | `5` | Partially | Only used in subscription creation retry loop. |
| `reconnect_delay_ms` | `2000` | Partially | Only used in subscription creation backoff. |

## Important caveats

- Stream config settings only apply on stream creation. Existing streams are not reconciled automatically.
- `storage_type`, `retention_policy`, and `discard_policy` values are validated during stream creation.
- Reconnect settings apply to subscription setup retry behavior.

## Recommended usage

- Treat `timeout_seconds` and `retry_attempts` as informational unless explicitly wired in your deployed version.
- Set `max_reconnect_attempts` to a positive value.
- Use stable policy values:
  - `storage_type`: `file` or `memory`
  - `retention_policy`: `limits`, `interest`, `workqueue`
  - `discard_policy`: `old`, `new`
