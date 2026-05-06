# API Errors

Aviso returns a consistent JSON error object for `4xx` and `5xx` responses.

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
  authorization helpers). The same UUID is also returned in the
  `X-Request-ID` HTTP response header on every response, including success
  responses and any framework-level fallback (404 for an unknown route, 405
  for a wrong method, etc.) where a body field is not under aviso's control.
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
