# Streaming Semantics

## `POST /api/v1/watch`

- If both `from_id` and `from_date` are omitted:
  - stream is live-only (new notifications from now onward).
- If exactly one replay parameter is present:
  - historical replay starts first, then transitions to live stream.
- If both are present:
  - request is rejected with `400`.

Example (live-only watch):

```bash
curl -N -X POST "http://localhost:8000/api/v1/watch" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "test_polygon",
    "identifier": {
      "time": "1200",
      "polygon": "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    }
  }'
```

## `POST /api/v1/replay`

- Requires exactly one replay start parameter:
  - `from_id` (sequence-based), or
  - `from_date` (time-based, RFC3339).
- If both are missing:
  - request is rejected with `400`.
- Endpoint returns historical replay stream and then closes.

Example (time-based replay):

```bash
curl -N -X POST "http://localhost:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "test_polygon",
    "identifier": {
      "time": "1200",
      "polygon": "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    },
    "from_date": "2026-02-14T13:00:00Z"
  }'
```

## `from_date` behavior

- Input must be RFC3339 datetime string with timezone.
- Parsed and normalized to UTC.
- JetStream replay uses start-time delivery policy when sequence is not provided.

## Parameter validity summary

- `watch`: zero or one replay parameter.
- `replay`: exactly one replay parameter.

## Backend requirement

Streaming endpoints (`watch` and `replay`) currently require `jetstream` backend.
