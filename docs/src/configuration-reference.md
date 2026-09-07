# Configuration Reference

This page documents runtime-relevant configuration fields and defaults.

## Topic Wire Format

- Topic wire subjects always use `.` as separator.
- Per-schema `topic.separator` is no longer used.
- Token values are percent-encoded for reserved chars (`.`, `*`, `>`, `%`)
  before writing to backend subjects.

See [Topic Encoding](./topic-encoding.md) for rules and examples.

## `application`

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`host`](#application-host) | none |
| [`port`](#application-port) | none |
| [`base_url`](#application-base-url) | `http://localhost` |
| [`static_files_path`](#application-static-files-path) | `/app/static` |

</div>
<details class="setting-panel" id="application-host">
<summary><code>host</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> <code>string</code></span>
</summary>

Bind address.

</details>
<details class="setting-panel" id="application-port">
<summary><code>port</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> <code>u16</code></span>
</summary>

Bind port.

</details>
<details class="setting-panel" id="application-base-url">
<summary><code>base_url</code>
<span class="setting-meta"><strong>Default:</strong> <code>http://localhost</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

Used in generated CloudEvent source links.

</details>
<details class="setting-panel" id="application-static-files-path">
<summary><code>static_files_path</code>
<span class="setting-meta"><strong>Default:</strong> <code>/app/static</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

Static asset root for homepage assets.

</details>
</div>

## `logging`

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`level`](#logging-level) | `info` |
| [`format`](#logging-format) | implementation default |

</div>
<details class="setting-panel" id="logging-level">
<summary><code>level</code>
<span class="setting-meta"><strong>Default:</strong> <code>info</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

One of `trace`, `debug`, `info`, `warn`, `error`. Unknown values fall back to
`info` instead of failing startup. Used as the application-wide level when
`RUST_LOG` is unset.

</details>
<details class="setting-panel" id="logging-format">
<summary><code>format</code>
<span class="setting-meta"><strong>Default:</strong> implementation default · <strong>Type:</strong> <code>string</code></span>
</summary>

Kept for compatibility; output is OTel-aligned JSON.

</details>
</div>

### Runtime override via `RUST_LOG`

If the `RUST_LOG` environment variable is set, it takes priority over
`logging.level` and gives the operator full
[`EnvFilter` directive syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives)
for runtime triage without a code change. Examples:

```bash
RUST_LOG=info,aviso_server=debug
RUST_LOG=warn,aviso_server::auth=trace
RUST_LOG=info,aviso_server::sse=debug,actix_web=warn
```

A malformed `RUST_LOG` value is reported on stderr at startup and the server
falls back to `logging.level`. The most common parse failures are an empty
target before `=` (for example `RUST_LOG==warn`) and a non-level value after
`=` (for example `RUST_LOG=info,aviso_server=verbose`).

A missing comma like `RUST_LOG=info aviso_server=debug` does **not** trigger the
fallback. `EnvFilter` parses the whole string as a single target name with a
space, and the directive ends up matching nothing instead of failing loudly. If
a `RUST_LOG` value looks correct but no logs appear, double-check the commas
first.

`RUST_LOG=""` (empty string) is treated as if `RUST_LOG` were unset and falls
back to `logging.level`. Without this guard `EnvFilter::try_new("")` silently
succeeds with a filter that matches nothing and silences the entire process.
This is a real failure mode under deployment systems that export unset variables
as empty strings, such as the Kubernetes downward API or docker-compose's
`${VAR:-}`.

When `RUST_LOG` is unset, the default filter combines `logging.level` with a
small set of mute directives so that framework internals do not flood
operational logs:

| Directive | Effect |
|---|---|
| `actix_web=warn` | Caps Actix-web request lifecycle logs at warn (worker started, accepting, etc.). |
| `actix_server=warn` | Caps Actix-server lifecycle logs at warn. |
| `async_nats=info` | Caps the NATS client at info; trace/debug per-message chatter stays off. |

These mute directives are pinned by unit tests, only apply when `RUST_LOG` is
unset, and only apply when the directive's level is **more restrictive** than
`logging.level`. With `logging.level=warn` or `logging.level=error` the
directives are skipped entirely so they never raise the per-target ceiling above
what the operator chose; with `logging.level=info` the two `actix_*=warn`
directives narrow framework chatter while `async_nats=info` is skipped (it would
be neutral); with `logging.level=debug` or `logging.level=trace` all three
directives apply. Setting `RUST_LOG` opts out of all of them and gives the
operator full directive control.

### Push-based export via `logging.otlp`

Logs are always written to stdout as OTel-aligned JSON. With an `otlp`
block the server additionally pushes every log record to an OpenTelemetry
collector over OTLP, for clusters where log collection is push-based
instead of scraping container output.

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`enabled`](#logging-otlp-enabled) | `false` |
| [`endpoint`](#logging-otlp-endpoint) | none |
| [`protocol`](#logging-otlp-protocol) | `"grpc"` |

</div>
<details class="setting-panel" id="logging-otlp-enabled">
<summary><code>enabled</code>
<span class="setting-meta"><strong>Default:</strong> <code>false</code> · <strong>Type:</strong> <code>bool</code></span>
</summary>

Turns OTLP log export on. Startup fails when enabled without an `endpoint`.

</details>
<details class="setting-panel" id="logging-otlp-endpoint">
<summary><code>endpoint</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> <code>string</code></span>
</summary>

Collector endpoint. A missing scheme defaults to `http://`. For
`protocol: http` the OTLP path `/v1/logs` is appended when absent.

</details>
<details class="setting-panel" id="logging-otlp-protocol">
<summary><code>protocol</code>
<span class="setting-meta"><strong>Default:</strong> <code>"grpc"</code> · <strong>Type:</strong> <code>"grpc"|"http"</code></span>
</summary>

Transport. Collectors conventionally listen on 4317 for gRPC and 4318 for HTTP.

</details>
</div>

```yaml
logging:
  level: info
  format: json
  otlp:
    enabled: true
    endpoint: "http://otel-collector.observability.svc:4317"
    protocol: grpc
```

Operational behavior:

- Export runs on a background batch thread with a bounded queue. A slow or
  unreachable collector never blocks request handling; overflow drops
  records from the export path only, and stdout remains complete.
- Export errors are reported by the SDK's internal diagnostics on stdout,
  so a broken collector connection is visible in `kubectl logs`.
- Export health is measurable on the Prometheus endpoint:
  `aviso_otlp_export_failures_total` counts failed batch exports and
  `aviso_otlp_suppressed_log_records_total` counts records withheld by
  the redaction guard. Alert on a sustained non-zero rate of either.
- The global filter (`logging.level` / `RUST_LOG`) applies to both sinks,
  so the collector receives the same event stream as stdout. The export
  transport's own targets (`opentelemetry*`, `tonic`, `hyper`, `h2`,
  `tower`, `reqwest`) are excluded from the export path to prevent
  feedback loops; they still appear on stdout.
- Exported records carry the same resource identity as stdout records
  (`service.name`, `service.version`, and `k8s.namespace.name` /
  `k8s.pod.name` when the corresponding environment variables are set).
- Redaction on the export path is stricter than stdout: record bodies get
  the same pattern redaction, but a record carrying a sensitive attribute
  key (`password`, `secret`, `token`, `authorization`, `api_key`) or a
  URL value with embedded credentials is withheld from export entirely.
  The stdout copy of the same record keeps field-level `[REDACTED]`
  markers, so no information is lost to operators.
- On shutdown the server flushes buffered records before exiting.

The endpoint can also be injected without a config file change via
environment overrides, for example
`AVISOSERVER_LOGGING__OTLP__ENDPOINT=http://collector:4317`.

## `auth`

Authentication is optional. When disabled (default), all API endpoints are
publicly accessible only if schemas do not define stream auth rules. Startup
fails if global auth is disabled while a schema sets `auth.required=true` or
non-empty `auth.read_roles`/`auth.write_roles`.

When enabled:

- Admin endpoints always require a valid JWT and an admin role.
- Stream endpoints (`notify`, `watch`, `replay`) enforce authentication only
  when the target schema has `auth.required: true`.
- Schema endpoints (`/api/v1/schema`) are always public.
- In `trusted_proxy` mode, Aviso validates `Authorization: Bearer <jwt>` locally
  with `jwt_secret`.

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`enabled`](#auth-enabled) | `false` |
| [`mode`](#auth-mode) | `"direct"` |
| [`auth_o_tron_url`](#auth-auth-o-tron-url) | `""` |
| [`jwt_secret`](#auth-jwt-secret) | `""` |
| [`admin_roles`](#auth-admin-roles) | `{}` |
| [`timeout_ms`](#auth-timeout-ms) | `5000` |

</div>
<details class="setting-panel" id="auth-enabled">
<summary><code>enabled</code>
<span class="setting-meta"><strong>Default:</strong> <code>false</code> · <strong>Type:</strong> <code>bool</code></span>
</summary>

Set to `true` to enable authentication.

</details>
<details class="setting-panel" id="auth-mode">
<summary><code>mode</code>
<span class="setting-meta"><strong>Default:</strong> <code>"direct"</code> · <strong>Type:</strong> <code>"direct"|"trusted_proxy"</code></span>
</summary>

`direct`: forward credentials to auth-o-tron. `trusted_proxy`: validate
forwarded JWT locally.

</details>
<details class="setting-panel" id="auth-auth-o-tron-url">
<summary><code>auth_o_tron_url</code>
<span class="setting-meta"><strong>Default:</strong> <code>""</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

auth-o-tron base URL. Required when `enabled=true` and `mode=direct`.

</details>
<details class="setting-panel" id="auth-jwt-secret">
<summary><code>jwt_secret</code>
<span class="setting-meta"><strong>Default:</strong> <code>""</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

Shared HMAC secret for JWT validation. Required when `enabled=true`. Not
exposed via `/api/v1/schema` endpoints and redacted when auth settings are
serialized or logged.

</details>
<details class="setting-panel" id="auth-admin-roles">
<summary><code>admin_roles</code>
<span class="setting-meta"><strong>Default:</strong> <code>{}</code> · <strong>Type:</strong> <code>map&lt;string, string[]&gt;</code></span>
</summary>

Realm-scoped roles for admin endpoints (`/api/v1/admin/*`). Must contain at
least one realm with non-empty roles when `enabled=true`.

</details>
<details class="setting-panel" id="auth-timeout-ms">
<summary><code>timeout_ms</code>
<span class="setting-meta"><strong>Default:</strong> <code>5000</code> · <strong>Type:</strong> <code>u64</code></span>
</summary>

Timeout for auth-o-tron requests (milliseconds). Must be `> 0`.

</details>
</div>

### Per-stream auth (`notification_schema.<event_type>.auth`)

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`required`](#notification-schema-event-type-auth-required) | (none) |
| [`read_roles`](#notification-schema-event-type-auth-read-roles) | (none) |
| [`write_roles`](#notification-schema-event-type-auth-write-roles) | (none) |
| [`plugins`](#notification-schema-event-type-auth-plugins) | (none) |

</div>
<details class="setting-panel" id="notification-schema-event-type-auth-required">
<summary><code>required</code>
<span class="setting-meta"><strong>Default:</strong> (none) · <strong>Type:</strong> <code>bool</code></span>
</summary>

Must be explicitly set whenever an `auth` block is present. When `true`, the
stream requires authentication.

</details>
<details class="setting-panel" id="notification-schema-event-type-auth-read-roles">
<summary><code>read_roles</code>
<span class="setting-meta"><strong>Default:</strong> (none) · <strong>Type:</strong> <code>map&lt;string, string[]&gt;</code></span>
</summary>

Realm-scoped roles for read access (watch/replay). When omitted, any
authenticated user can read. Use `["*"]` as the role list to grant realm-wide
access.

</details>
<details class="setting-panel" id="notification-schema-event-type-auth-write-roles">
<summary><code>write_roles</code>
<span class="setting-meta"><strong>Default:</strong> (none) · <strong>Type:</strong> <code>map&lt;string, string[]&gt;</code></span>
</summary>

Realm-scoped roles for write access (notify). When omitted, only users matching
global `admin_roles` can write. Use `["*"]` as the role list to grant
realm-wide access.

</details>
<details class="setting-panel" id="notification-schema-event-type-auth-plugins">
<summary><code>plugins</code>
<span class="setting-meta"><strong>Default:</strong> (none) · <strong>Type:</strong> <code>string[]</code></span>
</summary>

Optional list of authorization plugins to run after role-based checks.
Currently supported: `"ecpds"` (requires `--features ecpds` build). On a build
without the required feature, startup fails with a clear error pointing at
the offending stream. (Silent skip would widen access.) Empty `plugins: []`
is rejected; omit the field instead. Plugins only run when `auth.required`
is `true`.

</details>
</div>

See [Authentication](./authentication.md) for detailed setup, client usage, and
error responses.

## `ecpds`

Optional ECPDS destination authorization, available when built with
`--features ecpds`. Add `"ecpds"` to a stream's `auth.plugins` list to check
destination access on watch and replay requests.

| Setting | Default |
|---|---|
| [`username`](#ecpds-username) | none |
| [`password`](#ecpds-password) | none |
| [`servers`](#ecpds-servers) | none |
| [`match_key`](#ecpds-match-key) | none |
| [`target_field`](#ecpds-target-field) | `"name"` |
| [`cache_ttl_seconds`](#ecpds-cache-ttl) | `300` |
| [`max_entries`](#ecpds-max-entries) | `10000` |
| [`request_timeout_seconds`](#ecpds-request-timeout) | `30` |
| [`connect_timeout_seconds`](#ecpds-connect-timeout) | `5` |
| [`partial_outage_policy`](#ecpds-outage-policy) | `"strict"` |

<div class="settings-reference">
<details class="setting-panel" id="ecpds-username">
<summary><code>username</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> nonempty string</span>
</summary>

Service account username used for HTTP Basic Auth to ECPDS.

</details>
<details class="setting-panel" id="ecpds-password">
<summary><code>password</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> nonempty string</span>
</summary>

Service account password used for HTTP Basic Auth to ECPDS. It is redacted
in configuration debug output. The schema discovery API does not expose the
top-level `ecpds` settings.

</details>
<details class="setting-panel" id="ecpds-servers">
<summary><code>servers</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> list of URL strings</span>
</summary>

Use HTTPS to protect credentials and destination lookups. HTTP is accepted
only for local testing with `127.0.0.1`, `[::1]`, or `localhost`; other HTTP
addresses fail startup validation.

`servers` is a list of base URL strings, without query strings or fragments.
Path prefixes such as `https://proxy.example/ecpds-api/` are supported. Aviso
appends `/ecpds/v1/destination/list?id=<username>` to each base URL.

</details>
<details class="setting-panel" id="ecpds-match-key">
<summary><code>match_key</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> string</span>
</summary>

Set `match_key` to an ordinary identifier such as `destination`, declared in
the schema with `required: true`. The name must not contain whitespace, `/`,
or NUL. When the schema defines a topic, include this field in
`topic.key_order` so delivery is filtered by the authorized destination.

Spatial identifiers cannot be match keys: `PolygonHandler`,
`PointCloudHandler`, and the field name `polygon` are not allowed. Spatial
matching does not enforce access to an exact destination value.

</details>
<details class="setting-panel" id="ecpds-target-field">
<summary><code>target_field</code>
<span class="setting-meta"><strong>Default:</strong> <code>"name"</code> · <strong>Type:</strong> string</span>
</summary>

Selects a JSON field from each ECPDS destination record.
Records missing that field are skipped. To investigate missing destinations,
set `RUST_LOG=info,aviso_ecpds=debug` and look for
`auth.ecpds.fetch.skipped_record` events.

</details>
<details class="setting-panel" id="ecpds-cache-ttl">
<summary><code>cache_ttl_seconds</code>
<span class="setting-meta"><strong>Default:</strong> <code>300</code> · <strong>Unit:</strong> seconds · <strong>Minimum:</strong> <code>1</code></span>
</summary>

How long to cache a user's destination list before fetching it again.
Use a whole number of seconds.

</details>
<details class="setting-panel" id="ecpds-max-entries">
<summary><code>max_entries</code>
<span class="setting-meta"><strong>Default:</strong> <code>10000</code> · <strong>Unit:</strong> users · <strong>Minimum:</strong> <code>1</code></span>
</summary>

Maximum number of users in the destination cache. Use a whole number. The
cache uses TinyLFU eviction when it needs to make room.

</details>
<details class="setting-panel" id="ecpds-request-timeout">
<summary><code>request_timeout_seconds</code>
<span class="setting-meta"><strong>Default:</strong> <code>30</code> · <strong>Unit:</strong> seconds · <strong>Minimum:</strong> <code>1</code></span>
</summary>

Maximum time for the whole ECPDS request, from DNS lookup through reading
the response body. Use a whole number of seconds.

</details>
<details class="setting-panel" id="ecpds-connect-timeout">
<summary><code>connect_timeout_seconds</code>
<span class="setting-meta"><strong>Default:</strong> <code>5</code> · <strong>Unit:</strong> seconds · <strong>Minimum:</strong> <code>1</code></span>
</summary>

Maximum time to establish the connection, including TCP and TLS. This counts
toward the total request timeout; it is not extra time. Use a whole number
of seconds.

</details>
<details class="setting-panel" id="ecpds-outage-policy">
<summary><code>partial_outage_policy</code>
<span class="setting-meta"><strong>Default:</strong> <code>"strict"</code> · <strong>Values:</strong> <code>"strict"</code>, <code>"any_success"</code></span>
</summary>

Controls what happens when an ECPDS server is unavailable:

- `strict`: every configured server must respond successfully. If any fails,
  the destination lookup fails with HTTP 503.
- `any_success`: combine destinations from the servers that respond
  successfully. The lookup fails if none succeeds.

In both modes, Aviso combines the returned destination lists. With
`any_success`, destinations known only to an unavailable server may be
missing. See [Partial outage policy](./authentication.md#partial-outage-policy)
for the trade-off.

</details>
</div>

See
[ECPDS Destination Authorization](./authentication.md#ecpds-destination-authorization)
for setup and runtime behavior, and the [ECPDS runbook](./ecpds-runbook.md)
for troubleshooting.

## `metrics`

Optional Prometheus metrics endpoint. When enabled, a separate HTTP server
serves `/metrics` on an internal port for scraping by Prometheus/ServiceMonitor.
This keeps metrics isolated from the public API.

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`enabled`](#metrics-enabled) | `false` |
| [`host`](#metrics-host) | `"127.0.0.1"` |
| [`port`](#metrics-port) | none |

</div>
<details class="setting-panel" id="metrics-enabled">
<summary><code>enabled</code>
<span class="setting-meta"><strong>Default:</strong> <code>false</code> · <strong>Type:</strong> <code>bool</code></span>
</summary>

Enable the metrics endpoint.

</details>
<details class="setting-panel" id="metrics-host">
<summary><code>host</code>
<span class="setting-meta"><strong>Default:</strong> <code>"127.0.0.1"</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

Bind address for the metrics server. Defaults to loopback to avoid public
exposure.

</details>
<details class="setting-panel" id="metrics-port">
<summary><code>port</code>
<span class="setting-meta"><strong>Default:</strong> none · <strong>Type:</strong> <code>u16</code></span>
</summary>

Required when `enabled=true`. Must differ from `application.port`.

</details>
</div>

Exposed metrics:

<div class="settings-reference">
<div class="setting-index">

| Metric | Type |
|---|---|
| [`aviso_build_info`](#metric-aviso_build_info) | gauge |
| [`aviso_http_requests_total`](#metric-aviso_http_requests_total) | counter |
| [`aviso_http_request_duration_seconds`](#metric-aviso_http_request_duration_seconds) | histogram |
| [`aviso_http_requests_in_flight`](#metric-aviso_http_requests_in_flight) | gauge |
| [`aviso_backend_operations_total`](#metric-aviso_backend_operations_total) | counter |
| [`aviso_backend_operation_duration_seconds`](#metric-aviso_backend_operation_duration_seconds) | histogram |
| [`aviso_notifications_total`](#metric-aviso_notifications_total) | counter |
| [`aviso_sse_connections_active`](#metric-aviso_sse_connections_active) | gauge |
| [`aviso_sse_connections_total`](#metric-aviso_sse_connections_total) | counter |
| [`aviso_sse_unique_users_active`](#metric-aviso_sse_unique_users_active) | gauge |
| [`aviso_sse_events_sent_total`](#metric-aviso_sse_events_sent_total) | counter |
| [`aviso_sse_stream_errors_total`](#metric-aviso_sse_stream_errors_total) | counter |
| [`aviso_sse_connection_duration_seconds`](#metric-aviso_sse_connection_duration_seconds) | histogram |
| [`aviso_auth_requests_total`](#metric-aviso_auth_requests_total) | counter |

</div>
<details class="setting-panel" id="metric-aviso_build_info">
<summary><code>aviso_build_info</code>
<span class="setting-meta"><strong>Type:</strong> gauge · <strong>Labels:</strong> <code>version</code></span>
</summary>

Constant `1` with the server version as a label; join on it in dashboards to
annotate deploys.

</details>
<details class="setting-panel" id="metric-aviso_http_requests_total">
<summary><code>aviso_http_requests_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>route</code>, <code>method</code>, <code>status_code</code></span>
</summary>

HTTP requests on the main server by matched route pattern (e.g.
`/api/v1/schema/{event_type}`). Reserved label values: unrouted requests (404
scans) collapse into `route="unmatched"`, requests failing with a service-level
error (no route information available) record `route="error"`, and non-standard
HTTP methods collapse into `method="other"`. The label is named `route` (not
`endpoint`) to avoid colliding with the Prometheus Operator target label
`endpoint`.

</details>
<details class="setting-panel" id="metric-aviso_http_request_duration_seconds">
<summary><code>aviso_http_request_duration_seconds</code>
<span class="setting-meta"><strong>Type:</strong> histogram · <strong>Labels:</strong> <code>route</code>, <code>method</code></span>
</summary>

Request duration until response headers are ready. For the SSE routes
(`/api/v1/watch`, `/api/v1/replay`) this is stream *setup* latency, not
connection lifetime; see `aviso_sse_connection_duration_seconds`.

</details>
<details class="setting-panel" id="metric-aviso_http_requests_in_flight">
<summary><code>aviso_http_requests_in_flight</code>
<span class="setting-meta"><strong>Type:</strong> gauge · <strong>Labels:</strong> <code>method</code></span>
</summary>

HTTP requests currently being processed, by method. Labelled by method only
because the matched route pattern is not known until routing completes (after
the request is already in flight). Distinguishes "slow because busy" from
"slow because a downstream/backend stalled".

</details>
<details class="setting-panel" id="metric-aviso_backend_operations_total">
<summary><code>aviso_backend_operations_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>backend</code>, <code>operation</code>, <code>outcome</code></span>
</summary>

Notification-backend operations at the trait boundary. `operation` ∈
{`publish`, `get_batch`, `wipe_stream`, `wipe_all`, `delete_message`}; `outcome`
∈ {`ok`, `error`}. `subscribe_to_topic` is excluded (its work happens lazily as
the stream is polled).

</details>
<details class="setting-panel" id="metric-aviso_backend_operation_duration_seconds">
<summary><code>aviso_backend_operation_duration_seconds</code>
<span class="setting-meta"><strong>Type:</strong> histogram · <strong>Labels:</strong> <code>backend</code>, <code>operation</code>, <code>outcome</code></span>
</summary>

Caller-observed backend operation latency (same labels as
`aviso_backend_operations_total`). This is the metric to watch when notification
throughput plateaus while pods are underused: it isolates backend
(NATS/JetStream) latency from app CPU.

</details>
<details class="setting-panel" id="metric-aviso_notifications_total">
<summary><code>aviso_notifications_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>event_type</code>, <code>status</code></span>
</summary>

Total notification requests. `status` ∈ {`success`, `error`, `rejected`};
requests failing before schema validation record `event_type="unknown"`.

</details>
<details class="setting-panel" id="metric-aviso_sse_connections_active">
<summary><code>aviso_sse_connections_active</code>
<span class="setting-meta"><strong>Type:</strong> gauge · <strong>Labels:</strong> <code>route</code>, <code>event_type</code></span>
</summary>

Currently active SSE connections. `route` ∈ {`/api/v1/watch`, `/api/v1/replay`}.

</details>
<details class="setting-panel" id="metric-aviso_sse_connections_total">
<summary><code>aviso_sse_connections_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>route</code>, <code>event_type</code></span>
</summary>

Total SSE connections opened.

</details>
<details class="setting-panel" id="metric-aviso_sse_unique_users_active">
<summary><code>aviso_sse_unique_users_active</code>
<span class="setting-meta"><strong>Type:</strong> gauge · <strong>Labels:</strong> <code>route</code></span>
</summary>

Distinct users with active SSE connections.

</details>
<details class="setting-panel" id="metric-aviso_sse_events_sent_total">
<summary><code>aviso_sse_events_sent_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>route</code>, <code>event_type</code></span>
</summary>

Notification events delivered to SSE clients. Heartbeats, control events, and
close frames are not counted.

</details>
<details class="setting-panel" id="metric-aviso_sse_stream_errors_total">
<summary><code>aviso_sse_stream_errors_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>route</code>, <code>event_type</code></span>
</summary>

Error events emitted into SSE streams after the response started (typed stream
errors and notification rendering failures); these are invisible to
`aviso_http_requests_total` because the stream already returned `200`.

</details>
<details class="setting-panel" id="metric-aviso_sse_connection_duration_seconds">
<summary><code>aviso_sse_connection_duration_seconds</code>
<span class="setting-meta"><strong>Type:</strong> histogram · <strong>Labels:</strong> <code>route</code></span>
</summary>

SSE connection lifetime, observed when the connection closes (buckets 1s-24h).
Long-lived open connections appear in `aviso_sse_connections_active`, not here,
until they close.

</details>
<details class="setting-panel" id="metric-aviso_auth_requests_total">
<summary><code>aviso_auth_requests_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>mode</code>, <code>outcome</code></span>
</summary>

Authentication attempts. `mode` ∈ {`direct`, `trusted_proxy`}; `outcome` ∈
{`success`, `unauthorized`, `forbidden`, `service_unavailable`}.

</details>
</div>

The SSE and HTTP request metrics share a `route` label whose values are real
route patterns (e.g. `/api/v1/watch`), so a single dashboard `route` variable
spans both. Like the ECPDS counters below, the bounded label combinations of
`aviso_auth_requests_total`, `aviso_notifications_total` (including one series
per configured stream), and `aviso_backend_operations_total` /
`aviso_backend_operation_duration_seconds` (per active backend) are
pre-initialised at zero on startup so `rate(...) > 0` alert rules evaluate
against existing series.

A binary built with `--features ecpds` registers the following five metrics. The
unlabelled counters and the gauge appear as Prometheus series at process
startup. The two labelled counters (`access_decisions_total`, `fetch_total`)
are pre-initialised at startup with every documented `outcome` value, so each
`outcome` label appears as a series at zero before any ECPDS traffic; this lets
alert rules of the form `rate(metric{outcome="error"}[5m]) > 0` start evaluating
on a known-zero baseline rather than on a missing series.

<div class="settings-reference">
<div class="setting-index">

| Metric | Type |
|---|---|
| [`aviso_ecpds_cache_hits_total`](#metric-aviso_ecpds_cache_hits_total) | counter |
| [`aviso_ecpds_cache_misses_total`](#metric-aviso_ecpds_cache_misses_total) | counter |
| [`aviso_ecpds_cache_size`](#metric-aviso_ecpds_cache_size) | gauge |
| [`aviso_ecpds_access_decisions_total`](#metric-aviso_ecpds_access_decisions_total) | counter |
| [`aviso_ecpds_fetch_total`](#metric-aviso_ecpds_fetch_total) | counter |

</div>
<details class="setting-panel" id="metric-aviso_ecpds_cache_hits_total">
<summary><code>aviso_ecpds_cache_hits_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> (none)</span>
</summary>

ECPDS destination cache hits (requests served from cache without an upstream
call).

</details>
<details class="setting-panel" id="metric-aviso_ecpds_cache_misses_total">
<summary><code>aviso_ecpds_cache_misses_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> (none)</span>
</summary>

ECPDS destination cache misses (requests not served from cache). Includes
coalesced waiters that did not trigger an upstream call themselves;
`aviso_ecpds_fetch_total` is the right metric for "actual upstream calls".

</details>
<details class="setting-panel" id="metric-aviso_ecpds_cache_size">
<summary><code>aviso_ecpds_cache_size</code>
<span class="setting-meta"><strong>Type:</strong> gauge · <strong>Labels:</strong> (none)</span>
</summary>

Number of usernames in the ECPDS destination cache, sampled from moka after
eviction passes. Expired entries are pruned by moka asynchronously, so this
gauge can briefly include not-yet-pruned expired entries until the next
pending-tasks run.

</details>
<details class="setting-panel" id="metric-aviso_ecpds_access_decisions_total">
<summary><code>aviso_ecpds_access_decisions_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>outcome</code></span>
</summary>

Access decisions. `outcome` ∈ {`allow`, `deny_destination`,
`deny_match_key_missing`, `unavailable`, `admin_bypass`, `error`}.

</details>
<details class="setting-panel" id="metric-aviso_ecpds_fetch_total">
<summary><code>aviso_ecpds_fetch_total</code>
<span class="setting-meta"><strong>Type:</strong> counter · <strong>Labels:</strong> <code>outcome</code></span>
</summary>

Upstream fetch outcomes (recorded once per access check whose request actually
ran the upstream call; coalesced waiters do not contribute). `outcome` ∈
{`success`, `http_401`, `http_403`, `http_4xx`, `http_5xx`, `invalid_response`,
`unreachable`}.

</details>
</div>

Process-level metrics (CPU, memory, open FDs) are automatically collected on
Linux.

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

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`nats_url`](#notification-backend-jetstream-nats-url) | `nats://localhost:4222` |
| [`token`](#notification-backend-jetstream-token) | `None` |
| [`timeout_seconds`](#notification-backend-jetstream-timeout-seconds) | `30` |
| [`retry_attempts`](#notification-backend-jetstream-retry-attempts) | `3` |
| [`max_messages`](#notification-backend-jetstream-max-messages) | `None` |
| [`max_bytes`](#notification-backend-jetstream-max-bytes) | `None` |
| [`retention_time`](#notification-backend-jetstream-retention-time) | `None` |
| [`storage_type`](#notification-backend-jetstream-storage-type) | `file` |
| [`replicas`](#notification-backend-jetstream-replicas) | `None` |
| [`retention_policy`](#notification-backend-jetstream-retention-policy) | `limits` |
| [`discard_policy`](#notification-backend-jetstream-discard-policy) | `old` |
| [`max_reconnect_attempts`](#notification-backend-jetstream-max-reconnect-attempts) | unlimited |
| [`reconnect_delay_ms`](#notification-backend-jetstream-reconnect-delay-ms) | `2000` |
| [`publish_retry_attempts`](#notification-backend-jetstream-publish-retry-attempts) | `5` |
| [`publish_retry_base_delay_ms`](#notification-backend-jetstream-publish-retry-base-delay-ms) | `150` |

</div>
<details class="setting-panel" id="notification-backend-jetstream-nats-url">
<summary><code>nats_url</code>
<span class="setting-meta"><strong>Default:</strong> <code>nats://localhost:4222</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

NATS connection URL.

</details>
<details class="setting-panel" id="notification-backend-jetstream-token">
<summary><code>token</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code> · <strong>Type:</strong> <code>string?</code></span>
</summary>

Token auth; `NATS_TOKEN` env fallback.

</details>
<details class="setting-panel" id="notification-backend-jetstream-timeout-seconds">
<summary><code>timeout_seconds</code>
<span class="setting-meta"><strong>Default:</strong> <code>30</code> · <strong>Type:</strong> <code>u64?</code></span>
</summary>

NATS connection timeout for each startup connect attempt (`> 0`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-retry-attempts">
<summary><code>retry_attempts</code>
<span class="setting-meta"><strong>Default:</strong> <code>3</code> · <strong>Type:</strong> <code>u32?</code></span>
</summary>

Startup connect attempts before backend init fails (`> 0`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-max-messages">
<summary><code>max_messages</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code> · <strong>Type:</strong> <code>i64?</code></span>
</summary>

Stream message cap.

</details>
<details class="setting-panel" id="notification-backend-jetstream-max-bytes">
<summary><code>max_bytes</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code> · <strong>Type:</strong> <code>i64?</code></span>
</summary>

Stream size cap in bytes.

</details>
<details class="setting-panel" id="notification-backend-jetstream-retention-time">
<summary><code>retention_time</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code> · <strong>Type:</strong> <code>string?</code></span>
</summary>

Default stream max age (`s`, `m`, `h`, `d`, `w`; for example `30d`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-storage-type">
<summary><code>storage_type</code>
<span class="setting-meta"><strong>Default:</strong> <code>file</code> · <strong>Type:</strong> <code>string?</code></span>
</summary>

`file` or `memory` (parsed as typed enum at config load).

</details>
<details class="setting-panel" id="notification-backend-jetstream-replicas">
<summary><code>replicas</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code> · <strong>Type:</strong> <code>usize?</code></span>
</summary>

Stream replicas.

</details>
<details class="setting-panel" id="notification-backend-jetstream-retention-policy">
<summary><code>retention_policy</code>
<span class="setting-meta"><strong>Default:</strong> <code>limits</code> · <strong>Type:</strong> <code>string?</code></span>
</summary>

`limits`/`interest`. `workqueue` fails startup because independent watch/replay
consumers are not supported.

</details>
<details class="setting-panel" id="notification-backend-jetstream-discard-policy">
<summary><code>discard_policy</code>
<span class="setting-meta"><strong>Default:</strong> <code>old</code> · <strong>Type:</strong> <code>string?</code></span>
</summary>

`old`/`new` (parsed as typed enum at config load).

</details>
<details class="setting-panel" id="notification-backend-jetstream-max-reconnect-attempts">
<summary><code>max_reconnect_attempts</code>
<span class="setting-meta"><strong>Default:</strong> unlimited · <strong>Type:</strong> <code>u32?</code></span>
</summary>

Mapped to NATS `max_reconnects`; unset and `0` both mean unlimited. A positive
value makes the client give up permanently once exhausted.

Subscription creation uses a bounded retry loop: unset means five attempts,
`0` means one attempt, and a positive value sets the attempt limit. Startup
connection attempts are controlled separately by `retry_attempts`.

</details>
<details class="setting-panel" id="notification-backend-jetstream-reconnect-delay-ms">
<summary><code>reconnect_delay_ms</code>
<span class="setting-meta"><strong>Default:</strong> <code>2000</code> · <strong>Type:</strong> <code>u64?</code></span>
</summary>

Reconnect delay and startup connect retry backoff (`> 0`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-publish-retry-attempts">
<summary><code>publish_retry_attempts</code>
<span class="setting-meta"><strong>Default:</strong> <code>5</code> · <strong>Type:</strong> <code>u32?</code></span>
</summary>

Retry attempts for transient publish `channel closed` failures (`> 0`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-publish-retry-base-delay-ms">
<summary><code>publish_retry_base_delay_ms</code>
<span class="setting-meta"><strong>Default:</strong> <code>150</code> · <strong>Type:</strong> <code>u64?</code></span>
</summary>

Base backoff in milliseconds for publish retries (`> 0`).

</details>
</div>

See [JetStream Backend](./backend-jetstream.md#configuration-reference) for
detailed behavior.

## `notification_schema_strict`

Controls how the server treats `event_type` values that are not declared in
`notification_schema`.

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`notification_schema_strict`](#setting-notification-schema-strict) | **derived** |

</div>
<details class="setting-panel" id="setting-notification-schema-strict">
<summary><code>notification_schema_strict</code>
<span class="setting-meta"><strong>Default:</strong> <strong>derived</strong> · <strong>Type:</strong> <code>bool?</code></span>
</summary>

When unset, the effective value is `true` if `notification_schema` is
non-empty, `false` otherwise. Set to `true` to force strict rejection even
with no schema (deny-all "drain" mode). Set to `false` to preserve the legacy
permissive generic fallback even with a declared schema; a startup warning is
emitted in that case.

</details>
</div>

In strict mode, `POST /api/v1/notification`, `POST /api/v1/watch`, and
`POST /api/v1/replay` reject any `event_type` not present in
`notification_schema` with `400 UNKNOWN_EVENT_TYPE`.
The error body is:

```json
{
  "code": "UNKNOWN_EVENT_TYPE",
  "error": "unknown_event_type",
  "message": "unknown event type 'X'",
  "configured_event_types": ["dissemination", "mars", "test_polygon"],
  "request_id": "<uuid>"
}
```

`configured_event_types` is sorted for stable diffing in client tooling.

The same flag also bounds Prometheus / tracing label cardinality. Whenever
**effective** strict mode is off (either `notification_schema_strict` is
explicitly `false`, or it is unset with an empty/absent `notification_schema`
so the startup default resolves to non-strict), a request whose `event_type`
is not in the schema reaches the generic-fallback path and has its recorded
`event_type` label collapsed to the literal `"generic"` instead of being
persisted as user-controlled input.

## `notification_schema.<event_type>.payload`

Schema-level payload contract for notify requests.

| Field | Type | Example | Notes |
|---|---|---|---|
| `required` | `bool` | `true` | When `true`, `/notification` rejects requests without `payload`. |

Behavior details and edge cases are documented in
[Payload Contract](./payload-contract.md).

## `notification_schema.<event_type>.storage_policy`

Optional per-schema storage settings validated at startup against selected
backend capabilities.

<div class="settings-reference">
<div class="setting-index">

| Setting | Example |
|---|---|
| [`retention_time`](#notification-schema-event-type-storage-policy-retention-time) | `7d`, `12h`, `30m` |
| [`max_messages`](#notification-schema-event-type-storage-policy-max-messages) | `100000` |
| [`max_size`](#notification-schema-event-type-storage-policy-max-size) | `512Mi`, `2G` |
| [`allow_duplicates`](#notification-schema-event-type-storage-policy-allow-duplicates) | `true` |
| [`compression`](#notification-schema-event-type-storage-policy-compression) | `true` |

</div>
<details class="setting-panel" id="notification-schema-event-type-storage-policy-retention-time">
<summary><code>retention_time</code>
<span class="setting-meta"><strong>Example:</strong> <code>7d</code>, <code>12h</code>, <code>30m</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

Duration literal (`s`, `m`, `h`, `d`, `w`).

</details>
<details class="setting-panel" id="notification-schema-event-type-storage-policy-max-messages">
<summary><code>max_messages</code>
<span class="setting-meta"><strong>Example:</strong> <code>100000</code> · <strong>Type:</strong> <code>integer</code></span>
</summary>

Must be `> 0`.

</details>
<details class="setting-panel" id="notification-schema-event-type-storage-policy-max-size">
<summary><code>max_size</code>
<span class="setting-meta"><strong>Example:</strong> <code>512Mi</code>, <code>2G</code> · <strong>Type:</strong> <code>string</code></span>
</summary>

Size literal (`K`, `Ki`, `M`, `Mi`, `G`, `Gi`, `T`, `Ti`).

</details>
<details class="setting-panel" id="notification-schema-event-type-storage-policy-allow-duplicates">
<summary><code>allow_duplicates</code>
<span class="setting-meta"><strong>Example:</strong> <code>true</code> · <strong>Type:</strong> <code>bool</code></span>
</summary>

Backend support is capability-gated.

</details>
<details class="setting-panel" id="notification-schema-event-type-storage-policy-compression">
<summary><code>compression</code>
<span class="setting-meta"><strong>Example:</strong> <code>true</code> · <strong>Type:</strong> <code>bool</code></span>
</summary>

Backend support is capability-gated.

</details>
</div>

Field behavior:

- `retention_time` overrides backend-level retention for the schema stream.
- `max_messages` overrides backend-level message cap for the schema stream.
- `max_size` overrides backend-level byte cap for the schema stream.
- `allow_duplicates = false` maps to one message per subject (latest kept);
  `true` removes this cap.
- `compression = true` enables stream compression when backend supports it.

Startup behavior:

- Schema storage policies inherit the backend `retention_policy`; there is no
  per-schema override. `workqueue` fails startup because it does not support
  independent watch/replay consumers. Existing streams are not migrated or
  deleted by this validation.
- Invalid `retention_time`/`max_size` format fails startup.
- Unsupported fields for selected backend fail startup.
- Validation happens before backend initialization.
- With `in_memory`, all `storage_policy` fields are currently unsupported
  (startup fails if provided).

Runtime application behavior:

- `storage_policy` is applied on stream create and reconciled for existing
  JetStream streams when those streams are accessed by Aviso.
- Aviso-managed stream subject binding is also reconciled to the expected
  `<base>.>` pattern.
- Mutable fields (retention/limits/compression/duplicates/replicas) are updated
  when drift is detected.
- Recreate stream(s) only when you need historical data physically rewritten
  with new settings.

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

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`sse_heartbeat_interval_sec`](#watch-endpoint-sse-heartbeat-interval-sec) | `30` |
| [`connection_max_duration_sec`](#watch-endpoint-connection-max-duration-sec) | `3600` |
| [`replay_batch_size`](#watch-endpoint-replay-batch-size) | `100` |
| [`max_historical_notifications`](#watch-endpoint-max-historical-notifications) | `10000` |
| [`replay_batch_delay_ms`](#watch-endpoint-replay-batch-delay-ms) | `100` |
| [`concurrent_notification_processing`](#watch-endpoint-concurrent-notification-processing) | `15` |

</div>
<details class="setting-panel" id="watch-endpoint-sse-heartbeat-interval-sec">
<summary><code>sse_heartbeat_interval_sec</code>
<span class="setting-meta"><strong>Default:</strong> <code>30</code> · <strong>Type:</strong> <code>u64</code></span>
</summary>

SSE heartbeat period.

</details>
<details class="setting-panel" id="watch-endpoint-connection-max-duration-sec">
<summary><code>connection_max_duration_sec</code>
<span class="setting-meta"><strong>Default:</strong> <code>3600</code> · <strong>Type:</strong> <code>u64</code></span>
</summary>

Maximum live watch duration.

</details>
<details class="setting-panel" id="watch-endpoint-replay-batch-size">
<summary><code>replay_batch_size</code>
<span class="setting-meta"><strong>Default:</strong> <code>100</code> · <strong>Type:</strong> <code>usize</code></span>
</summary>

Historical fetch batch size.

</details>
<details class="setting-panel" id="watch-endpoint-max-historical-notifications">
<summary><code>max_historical_notifications</code>
<span class="setting-meta"><strong>Default:</strong> <code>10000</code> · <strong>Type:</strong> <code>usize</code></span>
</summary>

Replay cap for historical delivery.

</details>
<details class="setting-panel" id="watch-endpoint-replay-batch-delay-ms">
<summary><code>replay_batch_delay_ms</code>
<span class="setting-meta"><strong>Default:</strong> <code>100</code> · <strong>Type:</strong> <code>u64</code></span>
</summary>

Delay between historical replay batches.

</details>
<details class="setting-panel" id="watch-endpoint-concurrent-notification-processing">
<summary><code>concurrent_notification_processing</code>
<span class="setting-meta"><strong>Default:</strong> <code>15</code> · <strong>Type:</strong> <code>usize</code></span>
</summary>

Live stream CloudEvent conversion concurrency.

</details>
</div>

## Custom config file path

Set `AVISOSERVER_CONFIG_FILE` to use a specific config file instead of the
default search cascade:

```bash
AVISOSERVER_CONFIG_FILE=/path/to/config.yaml cargo run
```

When set, only this file is loaded as a file source (startup fails if it does
not exist). The default locations (`./configuration/config.yaml`,
`/etc/aviso_server/config.yaml`, `$HOME/.aviso_server/config.yaml`) are
skipped. `AVISOSERVER_*` field-level overrides still apply on top.

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
