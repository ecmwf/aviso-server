# Configuration Reference

This page documents runtime-relevant configuration fields and defaults.

## `application`

| Field | Type | Default | Notes |
|---|---|---|---|
| `host` | `string` | none | Bind address. |
| `port` | `u16` | none | Bind port. |
| `base_url` | `string` | `http://localhost` | Used in generated CloudEvent source links. |
| `static_files_path` | `string` | `/app/static` | Static asset root for homepage assets. |

## `watch_endpoint`

| Field | Type | Default | Notes |
|---|---|---|---|
| `sse_heartbeat_interval_sec` | `u64` | `30` | SSE heartbeat period. |
| `connection_max_duration_sec` | `u64` | `3600` | Maximum live watch duration. |
| `replay_batch_size` | `usize` | `100` | Historical fetch batch size. |
| `max_historical_notifications` | `usize` | `10000` | Replay cap for historical delivery. |
| `replay_batch_delay_ms` | `u64` | `100` | Delay between historical replay batches. |
| `concurrent_notification_processing` | `usize` | `15` | Live stream CloudEvent conversion concurrency. |

## `logging`

| Field | Type | Default | Notes |
|---|---|---|---|
| `level` | `string` | implementation default | Example: `info`, `debug`, `warn`, `error`. |
| `format` | `string` | implementation default | Example: `bunyan`, `json`, `console`. |

## `notification_backend`

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | `string` | none | `jetstream` or `in_memory`. |
| `in_memory` | object | optional | Used when `kind = in_memory`. |
| `jetstream` | object | optional | Used when `kind = jetstream`. |

### `notification_backend.in_memory`

| Field | Type | Default | Notes |
|---|---|---|---|
| `max_history_per_topic` | `usize` | `1` | Retained messages per topic in memory. |
| `max_topics` | `usize` | `10000` | Max tracked topics before LRU-style eviction. |
| `enable_metrics` | `bool` | `false` | Enables extra internal metrics logs. |

See [InMemory Backend](./backend-in-memory.md) for operational caveats.

### `notification_backend.jetstream`

| Field | Type | Default | Runtime usage summary |
|---|---|---|---|
| `nats_url` | `string` | `nats://localhost:4222` | NATS connection URL. |
| `token` | `string?` | `None` | Token auth; `NATS_TOKEN` env fallback. |
| `timeout_seconds` | `u64?` | `30` | NATS connection timeout for each startup connect attempt. |
| `retry_attempts` | `u32?` | `3` | Startup connect attempts before backend init fails (`min=1`). |
| `max_messages` | `i64?` | `None` | Stream message cap. |
| `max_bytes` | `i64?` | `None` | Stream size cap in bytes. |
| `retention_days` | `u32?` | `None` | Converted to stream max age. |
| `storage_type` | `string?` | `file` | `file` or `memory`. |
| `replicas` | `usize?` | `None` | Stream replicas. |
| `retention_policy` | `string?` | `limits` | `limits`/`interest`/`workqueue`. |
| `discard_policy` | `string?` | `old` | `old`/`new`. |
| `enable_auto_reconnect` | `bool?` | `true` | Enables/disables NATS client reconnect behavior. |
| `max_reconnect_attempts` | `u32?` | `5` | Mapped to NATS `max_reconnects` (`0` => unlimited). |
| `reconnect_delay_ms` | `u64?` | `2000` | Reconnect delay and startup connect retry backoff. |

See [JetStream Settings](./jetstream-settings.md) and [JetStream Backend](./backend-jetstream.md) for detailed behavior.

## Environment override examples

```bash
AVISOSERVER_APPLICATION__HOST=0.0.0.0
AVISOSERVER_APPLICATION__PORT=8000
AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream
AVISOSERVER_NOTIFICATION_BACKEND__JETSTREAM__NATS_URL=nats://localhost:4222
AVISOSERVER_NOTIFICATION_BACKEND__JETSTREAM__TOKEN=secret
AVISOSERVER_WATCH_ENDPOINT__REPLAY_BATCH_SIZE=200
```
