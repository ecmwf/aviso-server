# Authentication

Authentication is optional. When enabled, Aviso supports two modes:

- **Direct** — Aviso forwards `Bearer` or `Basic` credentials to [auth-o-tron](https://github.com/ecmwf/auth-o-tron), which returns a signed JWT.
- **Trusted proxy** — an upstream reverse proxy authenticates the user and forwards a signed JWT; Aviso validates it locally.

## How It Works

1. Client sends credentials to Aviso.
2. Middleware resolves user identity:
   - **direct**: forwards the `Authorization` header to auth-o-tron `GET /authenticate` and receives a JWT back.
   - **trusted_proxy**: validates the forwarded `Authorization: Bearer <jwt>` locally using `jwt_secret`.
3. Username, realm, and roles are extracted from JWT claims and attached to the request.
4. Route handlers enforce per-stream auth rules on `notify`, `watch`, and `replay`.
5. Admin endpoints (`/api/v1/admin/*`) always require a valid JWT with an admin role.

Schema endpoints (`GET /api/v1/schema`, `GET /api/v1/schema/{event_type}`) are always publicly accessible, even when auth is enabled.

## Quick Start (Direct Mode)

### 1. Start auth-o-tron

```bash
# Foreground (Ctrl+C to stop):
./scripts/auth-o-tron-docker.sh start

# Background:
./scripts/auth-o-tron-docker.sh start --detach
```

By default this uses `scripts/example_auth_config.yaml`.
To use your own config:

```bash
AUTH_O_TRON_CONFIG_FILE=/path/to/auth-config.yaml ./scripts/auth-o-tron-docker.sh start
```

The bundled example config defines three local test users in realm `localrealm`:

| User | Password | Role |
|------|----------|------|
| `admin-user` | `admin-pass` | `admin` |
| `reader-user` | `reader-pass` | `reader` |
| `producer-user` | `producer-pass` | `producer` |

### 2. Enable auth in config

```yaml
auth:
  enabled: true
  mode: direct
  auth_o_tron_url: "http://localhost:8080"
  jwt_secret: "your-shared-secret"   # must match auth-o-tron jwt.secret
  admin_roles:
    localrealm: ["admin"]
  timeout_ms: 5000
```

Roles are realm-scoped: `admin_roles` maps each realm name to its authorized role list. A user must belong to a listed realm **and** hold one of that realm's roles.

### 3. Run aviso-server

Auth is now enforced for:

- Admin endpoints (`/api/v1/admin/*`) — always require auth + admin role.
- Stream endpoints (`/api/v1/notification`, `/api/v1/watch`, `/api/v1/replay`) — only when the target schema sets `auth.required: true`.

For full field-level documentation, see the [`auth` section in Configuration Reference](./configuration-reference.md#auth).

## Trusted Proxy Mode

Use `trusted_proxy` when Aviso sits behind a reverse proxy or API gateway that handles authentication. The proxy authenticates the user (via OIDC, SAML, etc.) and forwards a signed JWT to Aviso.

Aviso validates the forwarded `Authorization: Bearer <jwt>` locally using `jwt_secret`. Username and roles are read directly from JWT claims — no outbound call to auth-o-tron is made.

```yaml
auth:
  enabled: true
  mode: trusted_proxy
  jwt_secret: "shared-signing-secret"
  admin_roles:
    ecmwf: ["admin"]
```

`auth_o_tron_url` is not required in this mode.

## Per-Stream Authentication

Streams support separate **read** and **write** access controls. Read access governs `/watch` and `/replay`; write access governs `/notification`.

Configure authentication per stream in your notification schema:

```yaml
notification_schema:
  # Public — no auth section means anonymous access
  public_events:
    payload:
      required: true
    topic:
      base: "public"

  # Authenticated — any valid user can read, only admins can write
  internal_events:
    payload:
      required: true
    topic:
      base: "internal"
    auth:
      required: true

  # Separate read/write roles
  sensor_data:
    payload:
      required: true
    topic:
      base: "sensor"
    auth:
      required: true
      read_roles:
        internal: ["analyst", "consumer"]
        external: ["partner"]
      write_roles:
        internal: ["producer"]

  # Realm-wide read access using wildcard, restricted write
  shared_events:
    payload:
      required: true
    topic:
      base: "shared"
    auth:
      required: true
      read_roles:
        internal: ["*"]
        external: ["analyst"]
      write_roles:
        internal: ["producer", "operator"]
```

### Read vs. write access defaults

| `auth.required` | `read_roles` | `write_roles` | Read (watch/replay) | Write (notify) |
|---|---|---|---|---|
| `false` or omitted | — | — | Anyone | Anyone |
| `true` | omitted | omitted | Any authenticated user | Admins only |
| `true` | set | omitted | Must match `read_roles` | Admins only |
| `true` | omitted | set | Any authenticated user | Must match `write_roles` or be admin |
| `true` | set | set | Must match `read_roles` | Must match `write_roles` or be admin |

Admins (users matching global `admin_roles`) always have both read and write access.

### Role matching rules

Both `read_roles` and `write_roles` map realm names to role lists. A user's `realm` claim from the JWT must match a key in the map, and the user must hold at least one of that realm's listed roles.

- **Wildcard `"*"`** — use `["*"]` as the role list to grant access to all users from a realm, regardless of their specific roles.
- **Omitted role list** — when `read_roles` is omitted, any authenticated user can read. When `write_roles` is omitted, only admins can write.

When a per-stream `auth` block is present, `auth.required` must be explicitly set to either `true` or `false`.

## ECPDS Destination Authorization

When built with `--features ecpds`, Aviso supports an optional authorization plugin that checks whether a user has access to a specific ECPDS destination before allowing `watch` or `replay` requests. The plugin is read-only: it never runs on `notify`.

### Enabling the plugin

1. Build Aviso with the `ecpds` feature:

```bash
cargo build --release --features ecpds
```

   On a build without this feature, any YAML containing `plugins: ["ecpds"]` is **rejected at startup** with an error pointing at the offending stream. This is deliberate: silently skipping the plugin would widen access.

2. Add a top-level `ecpds` section to your config with ECPDS service credentials:

```yaml
ecpds:
  username: "ecpds-service-account"
  password: "service-password"
  servers:
    - "https://ecpds-primary.ecmwf.int"
    - "https://ecpds-secondary.ecmwf.int"
  match_key: "destination"
  target_field: "name"            # default: "name"
  cache_ttl_seconds: 300          # default: 300 (5 min)
  max_entries: 10000              # default: 10000
  request_timeout_seconds: 30     # default: 30
  connect_timeout_seconds: 5      # default: 5
  partial_outage_policy: strict   # default: strict; alternative: any_success
```

3. Enable the plugin on a stream by adding `plugins: ["ecpds"]` to its `auth` block. Minimal canonical shape:

```yaml
notification_schema:
  dissemination:
    payload:
      required: true
    topic:
      base: "diss"
      key_order: ["destination", "target", "class", "expver", "domain", "date", "time", "stream", "step"]
    identifier:
      destination:
        - type: StringHandler
          required: true              # MUST be required
      # ... other fields ...
    auth:
      required: true                  # MUST be true
      plugins: ["ecpds"]
```

`read_roles` is optional. If you want a realm-wide gate **before** ECPDS even runs (e.g. block users from realms you don't trust to query ECPDS in the first place), add it; if not, omit it and the plugin runs for every authenticated user.

The plugin requires (and startup validation enforces):

- `match_key` (default `"destination"`) is present in the schema's `topic.key_order`.
- The same field is marked `required: true` in the schema's `identifier`. Without this, an operator could deploy a schema where the destination value is optional and a client could bypass the check by simply omitting the field.
- `auth.required` is `true`. The plugin runs after standard stream auth, so plugins on a stream where `auth.required` is `false` would never execute.

### How it works at runtime

1. Standard role-based stream auth runs first. If it fails (missing token, wrong realm/role), the request fails before the ECPDS plugin sees it.
2. The plugin extracts the `match_key` value (e.g. `destination`) from the request's canonicalised identifier.
3. It looks up the user's destination list in an in-process cache. If absent, it queries the configured ECPDS servers in parallel, then merges per the [`partial_outage_policy`](#partial-outage-policy).
4. If the requested destination is in the user's list, the request proceeds. Otherwise, `403 Forbidden`.
5. Users matching the global `auth.admin_roles` bypass step 2-4 entirely.

### Partial-outage policy

When more than one ECPDS server is configured, the `partial_outage_policy` field decides what happens if servers disagree or a subset is down.

| Value | Behaviour | Operational implication |
|-------|-----------|-------------------------|
| `strict` (default) | Every configured server must reply successfully **and** return the same set of destinations. Any server failure or any divergence fails the lookup. | Confidentiality-preserving: if servers disagree (replication lag, partial outage), the stricter view wins. **Loss of one server = total ECPDS unavailability.** |
| `any_success` | The lookup succeeds if any one server replies. The destination list is the union across reachable servers. | Availability-preserving: a single reachable server keeps the plugin working. **An out-of-sync permissive server can widen access** until others come back. Divergence is logged at `warn`. |

The plugin emits `auth.ecpds.fetch.divergence` (`warn`) when servers disagree, in either policy. Operators should treat divergence as a replication issue to investigate at the ECPDS side.

### Error responses

| Code | HTTP Status | When |
|------|-------------|------|
| `FORBIDDEN` | `403` | User does not have access to the requested destination, or the required identifier field is missing. |
| `SERVICE_UNAVAILABLE` | `503` | The lookup failed under the active `partial_outage_policy`. The cause is in the structured tracing event `auth.ecpds.check.unavailable` and on the `aviso_ecpds_fetch_total{outcome=…}` metric (e.g. `unreachable`, `http_401`, `http_5xx`, `divergence`, `invalid_response`). |

### Caching

Destination lists are cached per-user for `cache_ttl_seconds` (default 300). The cache is bounded at `max_entries` (default 10 000) with TinyLFU eviction. Successful results are cached; **errors are not** — a transient outage doesn't extend itself by being cached.

The cache is **single-flight**: concurrent requests for the same uncached username produce exactly one upstream ECPDS call; the rest await its result. This protects ECPDS during SSE-reconnect storms.

The cache is **process-local**. Restarting Aviso clears it. Multiple replicas have independent caches.

### What is not checked

`notify` (write) is never gated by ECPDS. The plugin applies only to reads (`watch`, `replay`).

### No retries by design

Aviso does not retry failed ECPDS calls. A 503 is the signal to investigate ECPDS itself, not to bump timeouts. See the [ECPDS runbook](./ecpds-runbook.md) for triage steps.

For the full `ecpds` field reference, see the [`ecpds` section in Configuration Reference](./configuration-reference.md#ecpds). For metrics and tracing event names, see the [ECPDS runbook](./ecpds-runbook.md).

## Admin Endpoints

Admin endpoints always require authentication and one of the configured `admin_roles`, regardless of per-stream settings:

- `DELETE /api/v1/admin/notification/{id}`
- `DELETE /api/v1/admin/wipe/stream`
- `DELETE /api/v1/admin/wipe/all`

See [Admin Operations](./admin-operations.md) for request/response details.

## Disabling Authentication

```yaml
auth:
  enabled: false
```

Or omit the `auth` section entirely. When auth is disabled, all endpoints are publicly accessible.

Startup fails if global auth is disabled while any schema defines `auth.required: true` or non-empty `auth.read_roles`/`auth.write_roles`. Remove stream-level auth blocks before disabling global auth.

## Client Usage

### Bearer token (both modes)

```bash
# Watch an authenticated stream:
curl -N -H "Authorization: Bearer <jwt-token>" \
  -X POST http://localhost:8000/api/v1/watch \
  -H "Content-Type: application/json" \
  -d '{"event_type": "private_events", "identifier": {}}'
```

### Basic credentials (direct mode only)

```bash
# Notify with Basic auth:
curl -X POST http://localhost:8000/api/v1/notification \
  -u "admin-user:admin-pass" \
  -H "Content-Type: application/json" \
  -d '{"event_type": "ops_events", "identifier": {"event_type": "deploy"}, "payload": "ok"}'
```

In direct mode, Aviso forwards Basic credentials to auth-o-tron, which authenticates the user and returns a JWT. The response JWT is validated and used for authorization.

## Error Responses

Auth errors use a subset of the standard [API error shape](./api-errors.md#response-shape) with three fields (`code`, `error`, `message`; no `details`):

```json
{
  "code": "UNAUTHORIZED",
  "error": "unauthorized",
  "message": "Authorization header is required"
}
```

| Code | HTTP Status | When |
|------|-------------|------|
| `UNAUTHORIZED` | `401` | Missing `Authorization` header, invalid token format, expired or bad signature. |
| `FORBIDDEN` | `403` | Valid credentials but user lacks the required role for the stream or admin endpoint. |
| `SERVICE_UNAVAILABLE` | `503` | auth-o-tron is unreachable or returned an unexpected error (direct mode only). |

A `401` response includes a `WWW-Authenticate` header indicating the supported scheme (`Bearer` in trusted-proxy mode; `Bearer, Basic` in direct mode).
