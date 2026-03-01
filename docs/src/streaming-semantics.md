# Streaming Semantics

## `POST /api/v1/watch`

- If both `from_id` and `from_date` are omitted:
  - stream is live-only (new notifications from now onward).
- If exactly one replay parameter is present:
  - historical replay starts first, then transitions to live stream.
- If both are present:
  - request is rejected with `400`.
- Spatial filtering:
  - see [Spatial Filter Model](#spatial-filter-model) below.

## `POST /api/v1/replay`

- Requires exactly one replay start parameter:
  - `from_id` (sequence-based), or
  - `from_date` (time-based, flexible datetime/timestamp parsing).
- If both are missing:
  - request is rejected with `400`.
- Endpoint returns historical replay stream and then closes.
- Same spatial filter contract as `watch` (see [Spatial Filter Model](#spatial-filter-model)).

## Spatial Filter Model

Use this mental model:

- `identifier` picks candidate notifications by topic fields (`time`, `date`, etc.).
- spatial filters (`identifier.polygon` or `identifier.point`) optionally narrow that candidate set.

### Rules

- `identifier.polygon`:
  - do polygon-intersects-polygon filtering.
- `identifier.point`:
  - do point-inside-notification-polygon filtering.
- both `identifier.polygon` and `identifier.point`:
  - invalid request (`400`).
- neither `identifier.polygon` nor `identifier.point`:
  - no spatial narrowing; filtering uses non-spatial identifier fields only.

### Decision Table

| `identifier.polygon` | `identifier.point` | Result |
|---|---|---|
| provided | omitted | polygon intersection filter |
| omitted | provided | point-in-polygon filter |
| omitted | omitted | no spatial filter |
| provided | provided | `400 Bad Request` |

For practical request/response examples, see:

- [Practical Examples: Basic Notify/Watch/Replay](./practical-examples/basic-notify-watch-replay.md)
- [Practical Examples: Spatial Filtering](./practical-examples/spatial-filtering.md)

## Identifier Constraints (`watch`/`replay`)

For schema-backed event types, `identifier` fields support constraint objects in `watch`/`replay`.
Scalar values are still valid and are treated as `eq`.

Supported operators by handler type:

| Handler | Operators |
|---|---|
| `IntHandler` | `eq`, `in`, `gt`, `gte`, `lt`, `lte`, `between` |
| `FloatHandler` | `eq`, `in`, `gt`, `gte`, `lt`, `lte`, `between` |
| `EnumHandler` | `eq`, `in` |

`between` expects exactly two values `[min,max]` and is inclusive.
Float constraints accept only finite values (no `NaN`/`inf`).
For floats, `eq`/`in` use exact numeric equality; they do not apply a tolerance.
Constraint objects are rejected on `/notification`; notify accepts scalar identifier values only.

For end-to-end generic examples (with `curl` requests and expected outcomes), see:

- [Practical Examples: Constraint Filtering](./practical-examples/constraint-filtering.md)
- [Practical Examples: Spatial Filtering](./practical-examples/spatial-filtering.md)
- [Practical Examples: Replay Starting Points](./practical-examples/replay-starting-points.md)

## `from_date` behavior

- Input accepts these forms:
  - RFC3339 with timezone (for example `2025-01-15T10:00:00Z`, `2025-01-15T10:00:00+02:00`)
  - Space-separated datetime with timezone (for example `2025-01-15 10:00:00+00:00`)
  - Naive datetime interpreted as UTC (for example `2025-01-15 10:00:00`, `2025-01-15T10:00:00`)
  - Unix epoch seconds or milliseconds (for example `1740509903`, `1740509903710`)
- Numeric `from_date` values are interpreted by digit count:
  - up to `11` digits => unix seconds
  - `12` or more digits => unix milliseconds
- Parsed and normalized to UTC internally.
- JetStream replay uses start-time delivery policy when sequence is not provided.

## SSE Timestamp Format

- Control/heartbeat/close event timestamps are emitted in canonical UTC second precision:
  - `YYYY-MM-DDTHH:MM:SSZ`
- Example:
  - `2026-02-25T18:58:23Z`

## Start Point for Historical Events

When you request historical data, you must tell the server where to start.

You can choose one of these fields:

- `from_id`
  - Start from a message sequence number (inclusive).
  - Example: `from_id: "42"` means replay starts at sequence `42`.
  - Use this when you know the last sequence you processed.
- `from_date`
  - Start from a UTC timestamp (inclusive).
  - Examples:
    - `from_date: "2025-01-15T10:00:00Z"` (RFC3339)
    - `from_date: "2025-01-15 10:00:00+00:00"` (space-separated with timezone)
    - `from_date: "1740509903"` (unix seconds)
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

Topic wire format and reserved-character handling are documented in [Topic Encoding](./topic-encoding.md).

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

## Replay Payload Shape

- Replay/watch CloudEvent output always includes `data.payload`.
- If notify payload was omitted for an optional schema, replay/watch returns `data.payload = null`.
- Payload values are not reshaped by Aviso (for example scalar strings remain strings).

See [Payload Contract](./payload-contract.md) for full input/storage/output mapping.
