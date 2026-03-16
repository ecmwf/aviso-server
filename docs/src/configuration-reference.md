# Configuration Reference

This page documents runtime-relevant configuration fields and defaults.

## Topic Wire Format

- Topic wire subjects always use `.` as separator.
- Per-schema `topic.separator` is no longer used.
- Token values are percent-encoded for reserved chars (`.`, `*`, `>`, `%`) before writing to backend subjects.

See [Topic Encoding](./topic-encoding.md) for rules and examples.

## `application`

| Field | Type | Default | Notes |
|---|---|---|---|
| `host` | `string` | none | Bind address. |
| `port` | `u16` | none | Bind port. |
| `base_url` | `string` | `http://localhost` | Used in generated CloudEvent source links. |
| `static_files_path` | `string` | `/app/static` | Static asset root for homepage assets. |

## `logging`

| Field | Type | Default | Notes |
|---|---|---|---|
| `level` | `string` | implementation default | Example: `info`, `debug`, `warn`, `error`. |
| `format` | `string` | implementation default | Kept for compatibility; output is OTel-aligned JSON. |

## `auth`

Authentication is optional. When disabled (default), all API endpoints are publicly accessible only if schemas do not define stream auth rules. Startup fails if global auth is disabled while a schema sets `auth.required=true` or non-empty `auth.allowed_roles`.

When enabled:
- Admin endpoints always require a valid JWT and an admin role.
- Stream endpoints (`notify`, `watch`, `replay`) enforce authentication only when the target schema has `auth.required: true`.
- Schema endpoints (`/api/v1/schema`) are always public.
- In `trusted_proxy` mode, Aviso validates `Authorization: Bearer <jwt>` locally with `jwt_secret`.

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | `bool` | `false` | Set to `true` to enable authentication. |
| `mode` | `"direct"\|"trusted_proxy"` | `"direct"` | `direct`: forward credentials to auth-o-tron. `trusted_proxy`: validate forwarded JWT locally. |
| `auth_o_tron_url` | `string` | `""` | auth-o-tron base URL. Required when `enabled=true` and `mode=direct`. |
| `jwt_secret` | `string` | `""` | Shared HMAC secret for JWT validation. Required when `enabled=true`. Not exposed via `/api/v1/schema` endpoints and redacted when auth settings are serialized or logged. |
| `admin_roles` | `string[]` | `[]` | Roles allowed on admin endpoints (`/api/v1/admin/*`). Must be non-empty when `enabled=true`. |
| `timeout_ms` | `u64` | `5000` | Timeout for auth-o-tron requests (milliseconds). Must be `> 0`. |

### Per-stream auth (`notification_schema.<event_type>.auth`)

| Field | Type | Default | Notes |
|---|---|---|---|
| `required` | `bool` | — | Must be explicitly set whenever an `auth` block is present. When `true`, the stream requires authentication. |
| `allowed_roles` | `string[]` | `[]` | When non-empty, only users with a listed role can access the stream. Only enforced when `required: true`. |

See [Authentication](./authentication.md) for detailed setup, client usage, and error responses.

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
| `timeout_seconds` | `u64?` | `30` | NATS connection timeout for each startup connect attempt (`> 0`). |
| `retry_attempts` | `u32?` | `3` | Startup connect attempts before backend init fails (`> 0`). |
| `max_messages` | `i64?` | `None` | Stream message cap. |
| `max_bytes` | `i64?` | `None` | Stream size cap in bytes. |
| `retention_time` | `string?` | `None` | Default stream max age (`s`, `m`, `h`, `d`, `w`; for example `30d`). |
| `storage_type` | `string?` | `file` | `file` or `memory` (parsed as typed enum at config load). |
| `replicas` | `usize?` | `None` | Stream replicas. |
| `retention_policy` | `string?` | `limits` | `limits`/`interest`/`workqueue` (parsed as typed enum at config load). |
| `discard_policy` | `string?` | `old` | `old`/`new` (parsed as typed enum at config load). |
| `enable_auto_reconnect` | `bool?` | `true` | Enables/disables NATS client reconnect behavior. |
| `max_reconnect_attempts` | `u32?` | `5` | Mapped to NATS `max_reconnects` (`0` => unlimited). |
| `reconnect_delay_ms` | `u64?` | `2000` | Reconnect delay and startup connect retry backoff (`> 0`). |
| `publish_retry_attempts` | `u32?` | `5` | Retry attempts for transient publish `channel closed` failures (`> 0`). |
| `publish_retry_base_delay_ms` | `u64?` | `150` | Base backoff in milliseconds for publish retries (`> 0`). |

See [JetStream Backend](./backend-jetstream.md#configuration-reference) for detailed behavior.

## `notification_schema.<event_type>.payload`

Schema-level payload contract for notify requests.

| Field | Type | Example | Notes |
|---|---|---|---|
| `required` | `bool` | `true` | When `true`, `/notification` rejects requests without `payload`. |

Behavior details and edge cases are documented in [Payload Contract](./payload-contract.md).

## `notification_schema.<event_type>.storage_policy`

Optional per-schema storage settings validated at startup against selected backend capabilities.

| Field | Type | Example | Notes |
|---|---|---|---|
| `retention_time` | `string` | `7d`, `12h`, `30m` | Duration literal (`s`, `m`, `h`, `d`, `w`). |
| `max_messages` | `integer` | `100000` | Must be `> 0`. |
| `max_size` | `string` | `512Mi`, `2G` | Size literal (`K`, `Ki`, `M`, `Mi`, `G`, `Gi`, `T`, `Ti`). |
| `allow_duplicates` | `bool` | `true` | Backend support is capability-gated. |
| `compression` | `bool` | `true` | Backend support is capability-gated. |

Field behavior:

- `retention_time` overrides backend-level retention for the schema stream.
- `max_messages` overrides backend-level message cap for the schema stream.
- `max_size` overrides backend-level byte cap for the schema stream.
- `allow_duplicates = false` maps to one message per subject (latest kept); `true` removes this cap.
- `compression = true` enables stream compression when backend supports it.

Startup behavior:

- Invalid `retention_time`/`max_size` format fails startup.
- Unsupported fields for selected backend fail startup.
- Validation happens before backend initialization.
- With `in_memory`, all `storage_policy` fields are currently unsupported (startup fails if provided).

Runtime application behavior:

- `storage_policy` is applied on stream create and reconciled for existing JetStream streams
  when those streams are accessed by Aviso.
- Aviso-managed stream subject binding is also reconciled to the expected `<base>.>` pattern.
- Mutable fields (retention/limits/compression/duplicates/replicas) are updated when drift is detected.
- Recreate stream(s) only when you need historical data physically rewritten with new settings.

Example:

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
    publish_retry_attempts: 5
    publish_retry_base_delay_ms: 150

notification_schema:
  dissemination:
    topic:
      base: "diss"
      key_order: ["destination", "target", "class", "expver", "domain", "date", "time", "stream", "step"]
    storage_policy:
      retention_time: "7d"
      max_messages: 2000000
      max_size: "10Gi"
      allow_duplicates: true
      compression: true
```

## `watch_endpoint`

| Field | Type | Default | Notes |
|---|---|---|---|
| `sse_heartbeat_interval_sec` | `u64` | `30` | SSE heartbeat period. |
| `connection_max_duration_sec` | `u64` | `3600` | Maximum live watch duration. |
| `replay_batch_size` | `usize` | `100` | Historical fetch batch size. |
| `max_historical_notifications` | `usize` | `10000` | Replay cap for historical delivery. |
| `replay_batch_delay_ms` | `u64` | `100` | Delay between historical replay batches. |
| `concurrent_notification_processing` | `usize` | `15` | Live stream CloudEvent conversion concurrency. |

## Environment override examples

```bash
AVISOSERVER_APPLICATION__HOST=0.0.0.0
AVISOSERVER_APPLICATION__PORT=8000
AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream
AVISOSERVER_NOTIFICATION_BACKEND__JETSTREAM__NATS_URL=nats://localhost:4222
AVISOSERVER_NOTIFICATION_BACKEND__JETSTREAM__TOKEN=secret
AVISOSERVER_WATCH_ENDPOINT__REPLAY_BATCH_SIZE=200
AVISOSERVER_AUTH__ENABLED=true
AVISOSERVER_AUTH__JWT_SECRET=secret
```
