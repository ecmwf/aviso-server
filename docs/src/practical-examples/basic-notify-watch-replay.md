# Basic Notify/Watch/Replay

Uses the shared generic schema from [Practical Examples](./overview.md).

This page is the quickest way to understand the normal API flow.
You first publish (`notify`), then observe live updates (`watch`), then read history (`replay`).
If you are onboarding a new environment, start here before trying filters or admin operations.
Read the examples in order.

## 1) Notify

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "anomaly":"42.5"
    },
    "payload":{"note":"initial forecast"}
  }'
```

Expected:

- HTTP `200`
- required identifier keys must be present; optional keys may be omitted

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
