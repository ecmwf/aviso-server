# Admin Operations (Practical)

See full reference in [Admin Operations](../admin-operations.md).

These operations are for cleanup and recovery, not normal data flow.
Use delete when one bad record must be removed; use wipe when resetting a stream or environment.
Because these endpoints are destructive, validate IDs and stream names carefully before execution.

## Delete One Notification by ID

```bash
curl -X DELETE "http://127.0.0.1:8000/api/v1/admin/notification/extreme_event@42"
```

Expected:

- `200` if it exists
- `404` if stream/sequence does not exist
- `400` for invalid format

## Wipe One Stream

```bash
curl -X DELETE "http://127.0.0.1:8000/api/v1/admin/wipe/stream" \
  -H "Content-Type: application/json" \
  -d '{"stream_name":"extreme_event"}'
```

The name is case-insensitive and accepts the event type or the backend
stream name (`extreme_event` and `EXTREME_EVENT` target the same stream).

Expected:

- `200` if the stream exists
- `404` if no stream matches; the message lists the configured event types
- stream definition remains, messages are removed

## Wipe All Streams

```bash
curl -X DELETE "http://127.0.0.1:8000/api/v1/admin/wipe/all"
```

Expected:

- `200`
- all stream data is removed
