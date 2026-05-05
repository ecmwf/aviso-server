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

Authentication is optional. When disabled (default), all API endpoints are publicly accessible only if schemas do not define stream auth rules. Startup fails if global auth is disabled while a schema sets `auth.required=true` or non-empty `auth.read_roles`/`auth.write_roles`.

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
| `admin_roles` | `map<string, string[]>` | `{}` | Realm-scoped roles for admin endpoints (`/api/v1/admin/*`). Must contain at least one realm with non-empty roles when `enabled=true`. |
| `timeout_ms` | `u64` | `5000` | Timeout for auth-o-tron requests (milliseconds). Must be `> 0`. |

### Per-stream auth (`notification_schema.<event_type>.auth`)

| Field | Type | Default | Notes |
|---|---|---|---|
| `required` | `bool` | — | Must be explicitly set whenever an `auth` block is present. When `true`, the stream requires authentication. |
| `read_roles` | `map<string, string[]>` | — | Realm-scoped roles for read access (watch/replay). When omitted, any authenticated user can read. Use `["*"]` as the role list to grant realm-wide access. |
| `write_roles` | `map<string, string[]>` | — | Realm-scoped roles for write access (notify). When omitted, only users matching global `admin_roles` can write. Use `["*"]` as the role list to grant realm-wide access. |
| `plugins` | `string[]` | (none) | Optional list of authorization plugins to run after role-based checks. Currently supported: `"ecpds"` (requires `--features ecpds` build). On a build without the required feature, startup fails with a clear error pointing at the offending stream. (Silent skip would widen access.) Empty `plugins: []` is rejected; omit the field instead. Plugins only run when `auth.required` is `true`. |

See [Authentication](./authentication.md) for detailed setup, client usage, and error responses.

## `ecpds`

Optional ECPDS destination authorization. Only available when built with `--features ecpds`. When configured, streams can reference the `"ecpds"` plugin in their `auth.plugins` list to enforce destination-level access control on `watch` and `replay` requests.

| Field | Type | Default | Notes |
|---|---|---|---|
| `username` | `string` | none | Service account username used for HTTP Basic Auth to ECPDS. Must not be empty. |
| `password` | `string` | none | Service account password. Redacted to `[REDACTED]` in `Debug` output (and therefore in any structured-log dump of the configuration). Must not be empty. The `/api/v1/schema` endpoint never exposes the top-level `ecpds` block at all, only per-event identifier and payload fields, so the password is not reachable through it. |
| `servers` | `string[]` | none | List of ECPDS server base URLs. Each must parse as a `http://` or `https://` URL with no query string and no fragment. Path prefixes (e.g. `https://proxy.example/ecpds-api/`) are accepted. The plugin appends `/ecpds/v1/destination/list?id=<username>` itself. |
| `match_key` | `string` | none | Identifier field to match against the user's destination list (e.g. `"destination"`). Must be a single bare identifier name (no whitespace, `/` or NUL) and must be present in the schema's `identifier` with `required: true` (so the value is guaranteed before the plugin runs). It does NOT need to appear in `topic.key_order`; the plugin reads the value from the request's canonicalized identifier params, not from topic routing. |
| `target_field` | `string` | `"name"` | JSON field to extract from each ECPDS destination record. Records that lack this field are silently skipped (logged at `info` as `auth.ecpds.fetch.skipped_record`). |
| `cache_ttl_seconds` | `u64` | `300` | How long (in seconds) to cache a user's destination list before re-fetching. Must be `> 0`. |
| `max_entries` | `u64` | `10000` | Maximum number of distinct usernames held in the cache; eviction policy is moka's TinyLFU. Must be `> 0`. |
| `request_timeout_seconds` | `u64` | `30` | Total wall-clock timeout for a single ECPDS HTTP request, end of TLS handshake to end of response body. Must be `> 0`. |
| `connect_timeout_seconds` | `u64` | `5` | TCP + TLS handshake timeout for a single ECPDS HTTP request. Must be `> 0`. |
| `partial_outage_policy` | `"strict"\|"any_success"` | `"strict"` | How tolerant the merge is when one configured server fails. The destination list itself is always the union of per-server responses. `strict`: every server must respond successfully or the call fails with 503. `any_success`: take the union of whichever servers responded; only fails if no server responded. See [ECPDS Destination Authorization](./authentication.md#partial-outage-policy) for the failure-tolerance trade-off. |

See [ECPDS Destination Authorization](./authentication.md#ecpds-destination-authorization) for setup and runtime behavior, and the [ECPDS runbook](./ecpds-runbook.md) for operational triage.

## `metrics`

Optional Prometheus metrics endpoint. When enabled, a separate HTTP server serves `/metrics` on an internal port for scraping by Prometheus/ServiceMonitor. This keeps metrics isolated from the public API.

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | `bool` | `false` | Enable the metrics endpoint. |
| `host` | `string` | `"127.0.0.1"` | Bind address for the metrics server. Defaults to loopback to avoid public exposure. |
| `port` | `u16` | none | Required when `enabled=true`. Must differ from `application.port`. |

Exposed metrics:

| Metric | Type | Labels | Description |
|---|---|---|---|
| `aviso_notifications_total` | counter | `event_type`, `status` | Total notification requests. |
| `aviso_sse_connections_active` | gauge | `endpoint`, `event_type` | Currently active SSE connections. |
| `aviso_sse_connections_total` | counter | `endpoint`, `event_type` | Total SSE connections opened. |
| `aviso_sse_unique_users_active` | gauge | `endpoint` | Distinct users with active SSE connections. |
| `aviso_auth_requests_total` | counter | `mode`, `outcome` | Authentication attempts. |

A binary built with `--features ecpds` registers the following five metrics. The unlabelled counters and the gauge appear as Prometheus series at process startup. The two labelled counters (`access_decisions_total`, `fetch_total`) are pre-initialised at startup with every documented `outcome` value, so each `outcome` label appears as a series at zero before any ECPDS traffic; this lets alert rules of the form `rate(metric{outcome="error"}[5m]) > 0` start evaluating on a known-zero baseline rather than on a missing series.

| Metric | Type | Labels | Description |
|---|---|---|---|
| `aviso_ecpds_cache_hits_total` | counter | (none) | ECPDS destination cache hits (requests served from cache without an upstream call). |
| `aviso_ecpds_cache_misses_total` | counter | (none) | ECPDS destination cache misses (requests not served from cache). Includes coalesced waiters that did not trigger an upstream call themselves; `aviso_ecpds_fetch_total` is the right metric for "actual upstream calls". |
| `aviso_ecpds_cache_size` | gauge | (none) | Number of usernames in the ECPDS destination cache, sampled from moka after eviction passes. Expired entries are pruned by moka asynchronously, so this gauge can briefly include not-yet-pruned expired entries until the next pending-tasks run. |
| `aviso_ecpds_access_decisions_total` | counter | `outcome` | Access decisions. `outcome` ∈ {`allow`, `deny_destination`, `deny_match_key_missing`, `unavailable`, `admin_bypass`, `error`}. |
| `aviso_ecpds_fetch_total` | counter | `outcome` | Upstream fetch outcomes (recorded once per access check whose request actually ran the upstream call; coalesced waiters do not contribute). `outcome` ∈ {`success`, `http_401`, `http_403`, `http_4xx`, `http_5xx`, `invalid_response`, `unreachable`}. |

Process-level metrics (CPU, memory, open FDs) are automatically collected on Linux.

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

## Custom config file path

Set `AVISOSERVER_CONFIG_FILE` to use a specific config file instead of the default search cascade:

```bash
AVISOSERVER_CONFIG_FILE=/path/to/config.yaml cargo run
```

When set, only this file is loaded as a file source (startup fails if it does not exist). The default locations (`./configuration/config.yaml`, `/etc/aviso_server/config.yaml`, `$HOME/.aviso_server/config.yaml`) are skipped. `AVISOSERVER_*` field-level overrides still apply on top.

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
AVISOSERVER_METRICS__ENABLED=true
AVISOSERVER_METRICS__PORT=9090
```
