# Practical Examples

This section provides copy-paste examples for common workflows.

All examples use the same generic event schema so behavior is easy to compare.

## Shared Generic Schema

```yaml
notification_schema:
  extreme_event:
    topic:
      base: extreme_event
      key_order: [region, run_time, severity, anomaly, polygon]
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

## Notify Identifier Rule

A subtlety that catches every first-time reader: `POST /api/v1/notification` requires **every** identifier key declared in the schema, not just the ones marked `required: true`. The `required` flag relaxes value validation (empty strings are accepted); it does **not** make the key itself optional. The shared schema above declares five keys, so notify examples on the following pages include all five. Watch and replay are different: there, missing keys are treated as wildcards automatically.

Next:

- [Basic Notify/Watch/Replay](./basic-notify-watch-replay.md)
- [Spatial Filtering](./spatial-filtering.md)
- [Constraint Filtering](./constraint-filtering.md)
- [Replay Starting Points](./replay-starting-points.md)
- [Admin Operations](./admin-operations.md)
