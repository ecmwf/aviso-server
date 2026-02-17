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
    "from_date": "2025-01-15T10:00:00Z"
  }'
```

## `from_date` behavior

- Input must be RFC3339 datetime string with timezone.
- Parsed and normalized to UTC.
- JetStream replay uses start-time delivery policy when sequence is not provided.

## Start Point for Historical Events

When you request historical data, you must tell the server where to start.

You can choose one of these fields:

- `from_id`
  - Start from a message sequence number (inclusive).
  - Use this when you know the last sequence you processed.
- `from_date`
  - Start from a UTC timestamp (inclusive, RFC3339).
  - Use this when you want events from a specific time onward.

## Rules by Endpoint

- `watch`
  - You may omit both fields to get a live-only stream (no history).
  - You may provide exactly one field (`from_id` or `from_date`) to get history first, then live.
  - Providing both fields is invalid (`400`).
- `replay`
  - You must provide exactly one field (`from_id` or `from_date`).
  - Omitting both fields or providing both is invalid (`400`).

## Backend Behavior

Streaming endpoints (`watch` and `replay`) work with both backends.

- `in_memory`
  - Data exists only inside the running server process.
  - Restarting the server clears all history.
  - Replay returns only events still kept in memory on that instance.
  - In multi-instance deployments, each instance has separate history.
- `jetstream`
  - Data is persisted in NATS JetStream.
  - Replay survives server restarts (subject to JetStream retention settings).

## SSE Implementation Model

Internally, streaming is implemented as a typed pipeline:

1. Request parameters are validated and converted to a typed replay cursor:
   - `StartAt::LiveOnly`
   - `StartAt::Sequence(u64)`
   - `StartAt::Date(DateTime<Utc>)`
2. Replay/live producers emit typed frames (`StreamFrame`) rather than raw SSE strings:
   - control frames (connection/replay lifecycle)
   - notification frames (live or replay)
   - heartbeat frames
   - error frames
   - close frame
3. Lifecycle handling is applied once:
   - server shutdown
   - optional max connection duration
   - natural end-of-stream
4. A single renderer converts typed frames into SSE wire format.

This design keeps endpoint semantics stable while making lifecycle behavior explicit and easier to maintain.

### Close Event Reasons

The final `connection-closing` event can carry one of these reasons:

- `server_shutdown`
- `max_duration_reached`
- `end_of_stream`

`/replay` is finite by design, so normal completion uses `end_of_stream`.
