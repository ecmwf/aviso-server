# Point-Cloud Filtering

Point-cloud streams let a provider publish many locations in one notification. A
subscriber supplies a polygon and receives the notification when any cloud point
lies inside the polygon or on its boundary.

Coordinates always use latitude first, then longitude. The canonical JSON forms
are:

- point: `[lat,lon]`
- polygon: `[[lat,lon],...,[lat,lon]]`, with the first pair repeated last
- point cloud: `[[lat,lon],...]`, with at least one point

Point and polygon strings remain accepted by the HTTP API for compatibility.
Point clouds accept JSON arrays only. Emitted CloudEvents always use arrays for
all spatial identifier values.

## Schema

```yaml
notification_schema:
  observations:
    topic:
      base: observations
      key_order: [date]
    identifier:
      date:
        type: DateHandler
        canonical_format: "%Y%m%d"
        required: true
      point_cloud:
        type: PointCloudHandler
        required: true
        max_points: 10000
        description: >-
          Publishers provide point_cloud as [[latitude, longitude], ...].
          Watch and replay requests provide a closed polygon instead. The
          polygon satisfies this required field; subscribers must not send
          point_cloud.
    payload:
      required: true
```

The handler must use the reserved `point_cloud` key. A schema can contain only
one `PointCloudHandler`. The configured `max_points` defaults to 10,000 and
cannot exceed 10,000.

Do not put `point_cloud` in `topic.key_order`. Startup rejects that
configuration. Cloud coordinates stay in the `spatial_point_cloud` backend
metadata header. The subject contains only normal routing fields. Aviso also
stores a `spatial_bbox` header for coarse rejection before exact matching.

## Notify

The provider sends the cloud on `/notification`:

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"observations",
    "identifier":{
      "date":"20260826",
      "point_cloud":[
        [52.52,13.40],
        [48.14,11.58],
        [52.52,13.40]
      ]
    },
    "payload":{"source":"stations"}
  }'
```

Duplicate points are valid. Aviso preserves their order. Every latitude and
longitude must be finite. Latitude must be in `[-90, 90]`; longitude must be in
`[-180, 180]`.

The canonical point-cloud JSON is limited to 60 KiB. This conservative Aviso
interoperability limit is informed by NATS-backed header transport and
near-limit round-trip tests. It leaves room for Aviso's other metadata, but is
not a protocol-wide NATS header limit. The point-count limit is checked first.

## Watch Or Replay

Subscribers use the reserved `polygon` query field. They do not send
`point_cloud`:

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type":"observations",
    "identifier":{
      "date":"20260826",
      "polygon":[
        [52.40,13.20],
        [52.40,13.70],
        [52.70,13.70],
        [52.70,13.20],
        [52.40,13.20]
      ]
    },
    "from_id":"0"
  }'
```

The polygon satisfies a required `point_cloud` field for watch and replay. Aviso
first rejects non-overlapping bounding boxes. It then tests points in their
stored order and stops at the first match. Points on an edge or vertex match. A
cloud whose points are all outside does not match.

Requests are rejected when they send `point_cloud` to watch or replay, declare
`polygon` in a point-cloud schema, or combine incompatible spatial filters.

## CloudEvent Output

The event reconstructs the provider identifier as JSON:

```json
{
  "data": {
    "identifier": {
      "date": "20260826",
      "point_cloud": [
        [52.52, 13.4],
        [48.14, 11.58],
        [52.52, 13.4]
      ]
    }
  }
}
```

The request polygon is a filter. It is not substituted into the emitted
identifier.
