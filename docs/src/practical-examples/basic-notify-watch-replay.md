# Basic Notify/Watch/Replay

Uses the shared generic schema from [Practical Examples](./overview.md).

This page is the quickest way to understand the normal API flow.
You first publish (`notify`), then observe live updates (`watch`), then read history (`replay`).
If you are onboarding a new environment, start here before trying filters or admin operations.
Read the examples in order.

## 1) Notify

Notify requires every identifier key declared in the schema, including those marked `required: false`. The flag relaxes value validation (an empty string is accepted); it does **not** make the key itself optional. The shared schema declares five keys (`region`, `run_time`, `severity`, `anomaly`, `polygon`), so all five appear below.

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "anomaly":"42.5",
      "polygon":"(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    },
    "payload":{"note":"initial forecast"}
  }'
```

Expected: HTTP `200`. Omitting any of the five identifier keys returns `400` with `code: INVALID_NOTIFICATION_REQUEST`.

## 2) Watch (Live Only)

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/watch" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "anomaly":"42.5"
    }
  }'
```

Expected:

- HTTP `200`
- SSE starts with `connection_established`
- only new matching notifications arrive

## 3) Replay (Historical)

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "anomaly":"42.5"
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- SSE emits `replay_started`, replay events, `replay_completed`, then closes
