# Streaming Semantics

This page defines the exact behavior of the watch and replay endpoints,
including start points, spatial filtering, identifier constraints, and SSE lifecycle events.

---

## SSE Stream Lifecycle

Every streaming response (watch or replay) goes through a typed lifecycle:

```mermaid
stateDiagram-v2
    [*] --> Connected : SSE connection established
    Connected --> Replaying : from_id or from_date provided
    Connected --> Live : no replay parameters (watch only)
    Replaying --> Live : replay_completed (watch only)
    Replaying --> Closed : end_of_stream (replay only)
    Live --> Closed : max_duration_reached
    Live --> Closed : server_shutdown
    Closed --> [*]
```

Close reasons emitted in the final `connection_closing` SSE event:

| Reason | Trigger |
|---|---|
| `end_of_stream` | Replay finished (`/replay` endpoint, or watch replay phase if live subscribe fails) |
| `max_duration_reached` | `connection_max_duration_sec` elapsed on a watch stream |
| `server_shutdown` | Server is shutting down gracefully |

---

## `POST /api/v1/watch`

- If both `from_id` and `from_date` are omitted:
  - stream is live-only (new notifications from now onward).
- If exactly one replay parameter is present:
  - historical replay starts first, then transitions to live stream.
- If both are present:
  - request is rejected with `400`.

```mermaid
flowchart TD
    A["Watch Request"] --> B{"from_id or<br/>from_date?"}
    B -->|neither| C["Live-only stream"]
    B -->|exactly one| D["Historical replay<br/>then live stream"]
    B -->|both| E["400 Bad Request"]

    style E fill:#8b1a1a,color:#fff
    style C fill:#1a6b3a,color:#fff
    style D fill:#1a4d6b,color:#fff
```

---

## `POST /api/v1/replay`

- Requires exactly one replay start parameter:
  - `from_id` (sequence-based), or
  - `from_date` (time-based).
- If both are missing or both are present:
  - request is rejected with `400`.
- Stream closes with `end_of_stream` when history is exhausted.

---

## Start Point for Historical Events

`from_id` starts delivery from that sequence number (inclusive).
`from_date` accepts any of these formats:

| Format | Example |
|---|---|
| RFC3339 with timezone | `2025-01-15T10:00:00Z` |
| RFC3339 with offset | `2025-01-15T10:00:00+02:00` |
| Space-separated with timezone | `2025-01-15 10:00:00+00:00` |
| Naive datetime (interpreted as UTC) | `2025-01-15T10:00:00` |
| Unix seconds (≤ 11 digits) | `1740509903` |
| Unix milliseconds (≥ 12 digits) | `1740509903710` |

All inputs are normalized to UTC internally.

---

## Spatial Filter Model

Spatial filtering applies on top of the identifier field filters.
Think of it in two layers:

- **Non-spatial identifier fields** (`time`, `date`, `class`, etc.) narrow candidates by topic routing.
- **Spatial fields** (`polygon` or `point`) further narrow that candidate set geographically.

```mermaid
flowchart LR
    A["Candidate<br/>notifications"] -->|"non-spatial<br/>identifier filter"| B["Topic-matched<br/>subset"]
    B -->|"spatial filter<br/>if provided"| C["Final<br/>results"]
```

### Rules

| `identifier.polygon` | `identifier.point` | Result |
|---|---|---|
| provided | omitted | polygon-intersects-polygon filter |
| omitted | provided | point-inside-notification-polygon filter |
| omitted | omitted | no spatial filter |
| provided | provided | `400 Bad Request` |

- `identifier.polygon`: keep notifications whose stored polygon intersects the request polygon.
- `identifier.point`: keep notifications whose stored polygon contains the request point.
- Both together: invalid — request is rejected.

---

## Identifier Constraints (`watch` / `replay`)

For schema-backed event types, identifier fields in watch/replay requests accept
**constraint objects** instead of (or in addition to) scalar values.
A scalar value is treated as an implicit `eq` constraint.

Constraint objects are **rejected on `/notification`** — notify only accepts scalar values.

### Supported operators by field type

| Handler | Operators |
|---|---|
| `IntHandler` | `eq`, `in`, `gt`, `gte`, `lt`, `lte`, `between` |
| `FloatHandler` | `eq`, `in`, `gt`, `gte`, `lt`, `lte`, `between` |
| `EnumHandler` | `eq`, `in` |

### Notes

- `between` expects exactly two values `[min, max]` and is **inclusive** on both ends.
- Float constraints reject `NaN` and `inf` — only finite values are valid.
- Float `eq` and `in` use **exact** numeric equality; no tolerance window is applied.
- A constraint object must contain **exactly one operator** — combining operators in a single object is rejected.

### Examples

```json
{ "severity": { "gte": 5 } }
{ "severity": { "between": [3, 7] } }
{ "region":   { "in": ["north", "south"] } }
{ "anomaly":  { "lt": 50.0 } }
```

---

## Backend Behavior

| Backend | Historical replay | Live watch |
|---|---|---|
| `in_memory` | Node-local only; clears on restart | Node-local fan-out |
| `jetstream` | Durable; survives restarts | Cluster-wide fan-out |

---

## SSE Timestamp Format

All control, heartbeat, and close event timestamps use canonical UTC second precision:

```
YYYY-MM-DDTHH:MM:SSZ
```

Example: `2026-02-25T18:58:23Z`

---

## Replay Payload Shape

- Replay and watch CloudEvent output always includes `data.payload`.
- If a notify request omitted payload (optional schema), replay returns `data.payload = null`.
- Payload values are not reshaped — scalar strings remain strings, objects remain objects.

See [Payload Contract](./payload-contract.md) for the full input → storage → output mapping.

---

For end-to-end examples, see:

- [Basic Notify/Watch/Replay](./practical-examples/basic-notify-watch-replay.md)
- [Constraint Filtering](./practical-examples/constraint-filtering.md)
- [Spatial Filtering](./practical-examples/spatial-filtering.md)
- [Replay Starting Points](./practical-examples/replay-starting-points.md)
