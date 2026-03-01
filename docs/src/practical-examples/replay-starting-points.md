# Replay Starting Points

Uses the shared generic schema from [Practical Examples](./overview.md).

Replay start parameters control where historical delivery begins.
Choose `from_id` when you track sequence progress; choose `from_date` when you track wall-clock time.
These examples cover valid forms and the common invalid combinations that return `400`.
Use this page to validate client retry and resume logic.

## Replay from Sequence (`from_id`)

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"north","run_time":"1200","severity":"4","anomaly":"42.5"},
    "from_id":"10"
  }'
```

Expected:

- HTTP `200`
- replay starts from sequence `10` (inclusive)

## Replay from Time (`from_date`) RFC3339

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"north","run_time":"1200","severity":"4","anomaly":"42.5"},
    "from_date":"2026-03-01T12:00:00Z"
  }'
```

Expected:

- HTTP `200`
- replay starts from that UTC timestamp (inclusive)

## Replay from Time (`from_date`) Unix Seconds

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"north","run_time":"1200","severity":"4","anomaly":"42.5"},
    "from_date":"1740509903"
  }'
```

Expected:

- HTTP `200`

## Replay from Time (`from_date`) Unix Milliseconds

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"north","run_time":"1200","severity":"4","anomaly":"42.5"},
    "from_date":"1740509903710"
  }'
```

Expected:

- HTTP `200`

## Invalid Replay Start Combinations

### Missing Both `from_id` and `from_date`

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"north","run_time":"1200","severity":"4","anomaly":"42.5"}
  }'
```

Expected:

- HTTP `400`

### Both `from_id` and `from_date` Provided

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/watch" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"north","run_time":"1200","severity":"4","anomaly":"42.5"},
    "from_id":"5",
    "from_date":"2026-03-01T12:00:00Z"
  }'
```

Expected:

- HTTP `400`
