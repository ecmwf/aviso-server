# API Errors

Errors produced by aviso's own handlers use a consistent JSON error object
on `4xx` and `5xx` responses. A small number of `4xx` responses are
framework-level fallbacks rather than aviso-handled errors and use a
different shape; see [Framework-Level Fallbacks](#framework-level-fallbacks)
below.

## Response Shape

```json
{
  "code": "INVALID_REPLAY_REQUEST",
  "error": "Invalid Replay Request",
  "message": "Replay endpoint requires either from_id or from_date parameter...",
  "details": "Replay endpoint requires either from_id or from_date parameter...",
  "request_id": "0d4f6758-1ce3-4dda-a0f3-0ccf5fcb50d6"
}
```

Fields:

- `code`: stable machine-readable error code.
- `error`: human-readable error category.
- `message`: top-level failure message (safe for client display).
- `details`: deepest/root detail available.
- `request_id`: per-request UUID. The same value is also returned in the
  `X-Request-ID` HTTP response header and in every server-side log line for
  this request. Quoting it is the easiest way to ask the operator to look up
  the corresponding traces.

Notes:

- `error_chain` is logged server-side for diagnostics, but is not returned in API responses.
- `message` is always present.
- `details` is present on `4xx` and `5xx` errors emitted from the
  notification/watch/replay request path (parse, validation, processing, and
  SSE stream initialization). It is intentionally omitted from authentication
  errors (`401`/`403`/`503`) and from the streaming-auth helpers (forbidden,
  ECPDS service unavailable, internal misconfiguration), where the upstream
  service or authorization plugin does not provide a stable detailed message.
- `request_id` is present on every error response body produced by aviso's
  own handlers (notification, watch, replay, schema, admin, auth and ECPDS
  authorization helpers). The `X-Request-ID` HTTP response header carries
  the same UUID on **every** response (success, aviso error, or
  framework-level fallback); see
  [Framework-Level Fallbacks](#framework-level-fallbacks) for the
  fallback cases where a JSON body field is not available.
- SSE stream initialization failures additionally include `topic` (the
  decoded logical topic) so the operator can scope log queries faster.
- The admin endpoints (`/api/v1/admin/*`) and `POST /api/v1/notification`
  return typed responses where `request_id` is a struct field rather than a
  free-form JSON key, but the field name and value are the same.

## How to report a problem

Capture either of these and pass them to the operator:

1. The `X-Request-ID` HTTP response header. Visible to `curl -i`, browser
   devtools, every reverse proxy, and most log aggregators. Present on every
   response, success or failure.
2. The `request_id` field in any error response body. Same UUID as the
   header.

For streaming responses (`/api/v1/watch`, `/api/v1/replay`), the same UUID
also appears in the JSON `data:` payload of the very first event and in any
`error` or `connection-closing` event the stream emits before terminating.
The exact wire shape depends on the stream variant:

- For a live-only watch, the first event has SSE `event: live-notification`
  and a JSON body with `"type": "connection_established"`.
- For a stream that begins with replay, the first event has SSE
  `event: replay-control` and a JSON body with `"type": "replay_started"`.

In both cases the `request_id` field is in the JSON body alongside `type`.
This means a user who only sees the open-ended SSE body (no header parsing)
can still recover the id without running the request again. See
[Streaming Semantics](./streaming-semantics.md#request-id-correlation) for
the full event-by-event payload table, including the SSE `event:` versus
`data.type` distinction (relevant for clients using
`EventSource.addEventListener`).

## Error Telemetry Events

These `event_name` values are emitted in structured logs:

| Event Name | Level | Trigger |
|---|---|---|
| `api.request.parse.failed` | `warn` | JSON parse/shape/unknown-field failure before domain validation. |
| `api.request.validation.failed` | `warn` | Domain/request validation failure (`400`). |
| `api.request.processing.failed` | `error` | Server-side processing/storage failure (`500`). |
| `stream.sse.initialization.failed` | `error` | Replay/watch SSE initialization failure (`500`). |

Every event carries `request_id`. The formatter additionally propagates `event_type` and `topic` from the surrounding request span when the handler has recorded them on the span before emitting the error log. Whether they appear depends on which step failed:

- `api.request.parse.failed` never carries `event_type` or `topic`. The request body is rejected before either is known.
- `api.request.validation.failed` sometimes carries `event_type`. Validation steps that run after the handler has parsed the schema (notify-side `process_notification_request` failures) include it. Steps that run before, namely the watch/replay request validator and the notify-side endpoint-mismatch check, do not.
- `api.request.processing.failed` carries `event_type`. Storage-write failures additionally carry `topic`.
- `stream.sse.initialization.failed` carries both `event_type` and `topic`.

In all cases, filter on `request_id` first; treat `event_type` and `topic` as auxiliary filters where present.

## Error Code Reference

| Code | HTTP Status | Meaning |
|---|---|---|
| `INVALID_JSON` | `400` | Request body is not valid JSON. |
| `UNKNOWN_FIELD` | `400` | Request contains fields outside API contract. |
| `INVALID_REQUEST_SHAPE` | `400` | JSON structure cannot be deserialized into request model. |
| `INVALID_NOTIFICATION_REQUEST` | `400` | Notification request failed business validation. |
| `INVALID_WATCH_REQUEST` | `400` | Watch request failed validation (replay/spatial/schema rules). |
| `INVALID_REPLAY_REQUEST` | `400` | Replay request failed validation (start cursor/spatial/schema rules). |
| `UNAUTHORIZED` | `401` | Missing or invalid credentials (no token, bad format, expired, bad signature). |
| `FORBIDDEN` | `403` | Valid credentials but user lacks the required role. |
| `NOTIFICATION_PROCESSING_FAILED` | `500` | Notification processing pipeline failed before storage. |
| `NOTIFICATION_STORAGE_FAILED` | `500` | Backend write operation failed. |
| `SSE_STREAM_INITIALIZATION_FAILED` | `500` | Replay/watch SSE stream could not be created. |
| `INTERNAL_ERROR` | `500` | Fallback internal error code (reserved). |
| `SERVICE_UNAVAILABLE` | `503` | Auth service (auth-o-tron) unreachable or returned an unexpected error. |

## Examples

Invalid replay request:

```json
{
  "code": "INVALID_REPLAY_REQUEST",
  "error": "Invalid Replay Request",
  "message": "Cannot specify both from_id and from_date...",
  "details": "Cannot specify both from_id and from_date...",
  "request_id": "0d4f6758-1ce3-4dda-a0f3-0ccf5fcb50d6"
}
```

Auth error (missing credentials on a protected stream).
Auth errors use four fields (`code`, `error`, `message`, `request_id`); `details` is not included:

```json
{
  "code": "UNAUTHORIZED",
  "error": "unauthorized",
  "message": "Authentication is required for this stream",
  "request_id": "0d4f6758-1ce3-4dda-a0f3-0ccf5fcb50d6"
}
```

SSE initialization failure:

```json
{
  "code": "SSE_STREAM_INITIALIZATION_FAILED",
  "error": "SSE stream creation failed",
  "message": "Failed to create stream consumer",
  "details": "nats connect failed: timeout",
  "topic": "test_polygon.*.1200",
  "request_id": "0d4f6758-1ce3-4dda-a0f3-0ccf5fcb50d6"
}
```

## Framework-Level Fallbacks

A few `4xx` responses are produced by the Actix HTTP framework itself
**before** any aviso handler runs, so the body shape is whatever the
framework defaults to (typically `text/plain` or empty) and not the JSON
object documented above:

| Status | Trigger |
|---|---|
| `404 Not Found` | Request path does not match any registered aviso route. |
| `405 Method Not Allowed` | Path matches an aviso route but the HTTP method does not. |
| `400 Bad Request` (rare) | Request fails framework-level checks (malformed Content-Length, etc.) before reaching aviso's body parsers. |

For these cases:

- `code`, `error`, `message`, `details`, and `request_id` JSON fields are
  **not** in the body.
- The `X-Request-ID` HTTP response header is still set on `404` and `405`
  (the middleware stack runs before Actix's default route-mismatch
  responses), so an operator can correlate the request with server logs
  by header alone. Errors raised even earlier in the HTTP stack (e.g.,
  malformed `Content-Length`, TLS handshake failure) bypass aviso's
  middleware entirely and produce no `X-Request-ID`; in those cases the
  request never reaches aviso, no log line is generated, and no
  correlation is possible.
- The aviso `code` reference table above does not apply.

If you need a stable JSON shape on these paths, hit a known-good route
(`GET /health` is the simplest); a `4xx` from there indicates an actual
aviso-handled error and follows the documented contract.
