# Practical Examples

This section provides copy-paste examples for common workflows.

All examples use the same generic event schema so behavior is easy to compare.

## Shared Generic Schema

```yaml
notification_schema:
  extreme_event:
    topic:
      base: extreme_event
      key_order: [region, run_time, severity, anomaly]
    identifier:
      region:
        description: "Geographic region label."
        type: EnumHandler
        values: ["north", "south", "east", "west"]
        required: true
      run_time:
        type: TimeHandler
        required: true
      severity:
        description: "Severity level from 1 to 7."
        type: IntHandler
        range: [1, 7]
        required: true
      anomaly:
        type: FloatHandler
        range: [0.0, 100.0]
        required: false
      polygon:
        type: PolygonHandler
        required: false
    payload:
      required: false
```

## Shared Assumptions

- Base URL: `http://127.0.0.1:8000`
- Content type: `application/json`
- Replay examples use `from_id` or `from_date` explicitly.

## Identifier Value Style

The examples in this section send scalar identifier values as JSON strings,
including numeric ones (`"severity":"4"`, `"anomaly":"42.5"`). The server
canonicalizes scalar values to strings. JSON numbers are accepted and produce
the same result. Constraint arguments (`{"gte":5}`, `{"between":[3,7]}`) use
JSON numbers because they are typed comparison operands.

## Notify Identifier Rule

`POST /api/v1/notification` requires every identifier key declared in the
schema, whether it has `required: true` or `required: false`. Every value must
pass its handler validation.

The flag matters on **watch** and **replay**. A missing key marked
`required: true` returns `400`. A missing key marked `required: false` becomes a
wildcard. Provided values and constraint objects still pass through handler
validation.

The shared schema above declares five keys, so notify examples on the following
pages include all five.

Next:

- [Basic Notify/Watch/Replay](./basic-notify-watch-replay.md)
- [Spatial Filtering](./spatial-filtering.md)
- [Constraint Filtering](./constraint-filtering.md)
- [Replay Starting Points](./replay-starting-points.md)
- [Admin Operations](./admin-operations.md)
