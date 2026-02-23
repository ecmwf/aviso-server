# API Errors

Aviso returns a consistent JSON error object for `4xx` and `5xx` responses.

## Response Shape

```json
{
  "code": "INVALID_REPLAY_REQUEST",
  "error": "Invalid Replay Request",
  "message": "Replay endpoint requires either from_id or from_date parameter...",
  "details": "Replay endpoint requires either from_id or from_date parameter..."
}
```

Fields:

- `code`: stable machine-readable error code.
- `error`: human-readable error category.
- `message`: top-level failure message (safe for client display).
- `details`: deepest/root detail available.

Notes:

- `error_chain` is logged server-side for diagnostics, but is not returned in API responses.
- `message` and `details` are always present for both `4xx` and `5xx` error responses.

## Error Telemetry Events

These `event_name` values are emitted in structured logs:

| Event Name | Level | Trigger |
|---|---|---|
| `api_request_parse_failed` | `warn` | JSON parse/shape/unknown-field failure before domain validation. |
| `api_request_validation_failed` | `warn` | Domain/request validation failure (`400`). |
| `api_request_processing_failed` | `error` | Server-side processing/storage failure (`500`). |
| `api_sse_stream_initialization_failed` | `error` | Replay/watch SSE initialization failure (`500`). |

For SSE setup failures, response also includes:

- `topic`: decoded logical topic.
- `request_id`: request correlation id.

## Error Code Reference

| Code | HTTP Status | Meaning |
|---|---|---|
| `INVALID_JSON` | `400` | Request body is not valid JSON. |
| `UNKNOWN_FIELD` | `400` | Request contains fields outside API contract. |
| `INVALID_REQUEST_SHAPE` | `400` | JSON structure cannot be deserialized into request model. |
| `INVALID_NOTIFICATION_REQUEST` | `400` | Notification request failed business validation. |
| `INVALID_WATCH_REQUEST` | `400` | Watch request failed validation (replay/spatial/schema rules). |
| `INVALID_REPLAY_REQUEST` | `400` | Replay request failed validation (start cursor/spatial/schema rules). |
| `NOTIFICATION_PROCESSING_FAILED` | `500` | Notification processing pipeline failed before storage. |
| `NOTIFICATION_STORAGE_FAILED` | `500` | Backend write operation failed. |
| `SSE_STREAM_INITIALIZATION_FAILED` | `500` | Replay/watch SSE stream could not be created. |
| `INTERNAL_ERROR` | `500` | Fallback internal error code (reserved). |

## Examples

Invalid replay request:

```json
{
  "code": "INVALID_REPLAY_REQUEST",
  "error": "Invalid Replay Request",
  "message": "Cannot specify both from_id and from_date...",
  "details": "Cannot specify both from_id and from_date..."
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
