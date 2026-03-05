# Constraint Filtering

Uses the shared generic schema from [Practical Examples](./overview.md).

Constraint filtering lets subscribers express conditions over identifier fields instead of exact values.
In practice, this is how you ask for ranges (`severity >= 5`), numeric bands, or enum subsets.
This page starts with seed data, then shows valid constraint requests, then common failure cases.
It is the best reference for building client-side filter payloads.

## Seed Notifications

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"north","run_time":"1200","severity":"3","anomaly":42.5},
    "payload":{"note":"seed-a"}
  }'

curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"south","run_time":"1200","severity":"6","anomaly":87.2},
    "payload":{"note":"seed-b"}
  }'
```

Expected:

- both return HTTP `200`

## Scalar Value (Implicit `eq`)

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{"region":"south","run_time":"1200","severity":6,"anomaly":87.2},
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- only `severity = 6` notifications match

## Integer Constraint (`gte`)

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":{"in":["north","south"]},
      "run_time":"1200",
      "severity":{"gte":5},
      "anomaly":87.2
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- includes `severity=6`
- excludes `severity=3`

## Float Constraint (`between`)

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"3",
      "anomaly":{"between":[40.0,50.0]}
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- includes `anomaly=42.5`

## Float `eq` Is Exact (No Tolerance)

Float `eq` and `in` are exact comparisons. This keeps behavior deterministic across replay/live
and avoids hidden tolerance windows.

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"3",
      "anomaly":{"eq":42.5}
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- only notifications with exactly `anomaly=42.5` match
- `NaN`/`inf` values are rejected by float validation/constraints

## Enum Constraint (`in`)

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/watch" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":{"in":["south","west"]},
      "run_time":"1200",
      "severity":"6",
      "anomaly":87.2
    }
  }'
```

Expected:

- HTTP `200`
- live notifications pass only for regions in `["south","west"]`

## Invalid: Two Operators in One Constraint Object

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":{"gte":4,"lt":7},
      "anomaly":42.5
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `400`
- message says constraint object must contain exactly one operator

## Invalid: Constraint Object on `/notification`

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":{"gte":4},
      "anomaly":42.5
    },
    "payload":{"note":"should-fail"}
  }'
```

Expected:

- HTTP `400`
