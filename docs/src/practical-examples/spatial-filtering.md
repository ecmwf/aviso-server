# Spatial Filtering

Uses the shared generic schema from [Practical Examples](./overview.md).

Spatial filtering has two modes:

- `identifier.polygon`: keep notifications whose polygon intersects the request polygon.
- `identifier.point`: keep notifications whose polygon contains the request point.

This matters when many notifications share similar non-spatial identifiers and you need geographic precision.

## Seed Notifications

These two notifications differ only by polygon shape.

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "polygon":"(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    },
    "payload":{"note":"poly-a"}
  }'

curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "polygon":"(10.0,10.0,10.2,10.0,10.2,10.2,10.0,10.2,10.0,10.0)"
    },
    "payload":{"note":"poly-b"}
  }'
```

Expected:

- both requests return HTTP `200`

## Replay with Polygon Intersection Filter

This request polygon intersects `poly-a` but not `poly-b`.

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "polygon":"(52.52,13.45,52.62,13.55,52.52,13.65,52.42,13.55,52.52,13.45)"
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- replay includes `poly-a`
- replay excludes `poly-b`

## Replay with Point Containment Filter

The point below is inside `poly-a` and outside `poly-b`.

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "point":"52.55,13.50"
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- replay includes `poly-a`
- replay excludes `poly-b`

## Replay Without Spatial Filter

No `polygon` and no `point` means no spatial narrowing.

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4"
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `200`
- replay may include both `poly-a` and `poly-b` because only non-spatial fields are applied

## Invalid: `polygon` and `point` Together

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"extreme_event",
    "identifier":{
      "region":"north",
      "run_time":"1200",
      "severity":"4",
      "polygon":"(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)",
      "point":"52.55,13.50"
    },
    "from_id":"1"
  }'
```

Expected:

- HTTP `400`
- validation error says both spatial filters cannot be used together
