# Admin Operations

Admin endpoints are destructive. Restrict access in production.

## Delete One Notification

`DELETE /api/v1/admin/notification/{notification_id}`

`notification_id` format:

- `<stream>@<sequence>` (canonical)
- `<event_type>@<sequence>` (alias; resolved through configured schema `topic.base`)

### How `notification_id` maps to your schema

Delete IDs use the stream key plus backend sequence number.

If your schema contains:

```yaml
notification_schema:
  mars:
    topic:
      base: "mars"
  dissemination:
    topic:
      base: "diss"
  test_polygon:
    topic:
      base: "polygon"
```

Then valid delete IDs include:

- `mars@42` (event type alias and stream key are same)
- `dissemination@42` (alias form, resolved to stream key `diss`)
- `diss@42` (canonical stream key form)
- `test_polygon@306` (alias form, resolved to stream key `polygon`)
- `polygon@306` (canonical stream key form)

### Example: replay ID then delete

Replay returns CloudEvent IDs like `mars@1`:

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "mars",
    "identifier": {
      "class": "od",
      "expver": "0001",
      "domain": "g",
      "date": "20250706",
      "time": "1200",
      "stream": "enfo",
      "step": "1"
    },
    "from_id": "1"
  }'
```

If one replayed event has `"id":"mars@1"`, delete it with:

```bash
curl -X DELETE "http://127.0.0.1:8000/api/v1/admin/notification/mars@1"
```

### Response behavior

- `200`: notification deleted.
- `404`: stream/sequence pair not found.
- `400`: invalid ID format (`<name>@<positive-integer>` required).

Invalid examples:

- `mars` (missing `@sequence`)
- `mars@0` (sequence must be > 0)
- `mars@abc` (sequence must be an integer)

## Wipe Endpoints

- `DELETE /api/v1/admin/wipe/stream`
- `DELETE /api/v1/admin/wipe/all`

These endpoints remove many messages at once and should be used with extreme caution.

## Wipe One Stream

`DELETE /api/v1/admin/wipe/stream`

Request body:

```json
{
  "stream_name": "MARS"
}
```

Example:

```bash
curl -X DELETE "http://127.0.0.1:8000/api/v1/admin/wipe/stream" \
  -H "Content-Type: application/json" \
  -d '{"stream_name":"MARS"}'
```

What it does:

- Removes all stored messages for the selected stream.
- Keeps the stream definition/configuration in place.
- New notifications can still be written to that stream immediately after wipe.

When to use:

- You want to reset one event family (`mars`, `diss`, `polygon`) without affecting others.

## Wipe All Streams

`DELETE /api/v1/admin/wipe/all`

Example:

```bash
curl -X DELETE "http://127.0.0.1:8000/api/v1/admin/wipe/all"
```

What it does:

- Removes all stored messages from all streams managed by the backend.
- Leaves service configuration intact, but data history is gone.

When to use:

- Local/dev reset before a fresh test run.
- Operational emergency cleanup where full history removal is intended.

## Which Admin Operation Should I Use?

- Delete one notification (`/admin/notification/{id}`):
  - Use when you know the exact sequence to remove.
- Wipe one stream (`/admin/wipe/stream`):
  - Use when one stream is polluted and others must remain untouched.
- Wipe all (`/admin/wipe/all`):
  - Use only when complete history reset is intended.

## Wipe Response Shape

Both wipe endpoints return:

```json
{
  "success": true,
  "message": "..."
}
```

Failure responses keep the same shape with `success: false` and an error message.
