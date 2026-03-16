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
