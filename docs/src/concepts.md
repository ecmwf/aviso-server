# Key Concepts

These are the core terms used throughout Aviso's documentation and API. Read this page before Getting Started to make the commands easier to follow.

---

## Event Type

An **event type** is a named category of notification, for example `extreme_event`, `sensor_alert`, or `data_ready`.

Every request to Aviso (notify, watch, or replay) targets exactly one event type.
The server uses the event type to:

- look up the matching schema (validation rules, required fields, topic ordering),
- route messages to the correct storage stream,
- apply the correct retention and storage policy.

```json
{ "event_type": "extreme_event", ... }
```

When a `notification_schema` is configured, Aviso is **strict by default**: any
`event_type` that is not in the schema is rejected with `400 UNKNOWN_EVENT_TYPE`
on `/notification`, `/watch`, and `/replay`. The error body lists the configured
event types so clients can self-correct.

When `notification_schema` is empty or absent (no schema declared at all), Aviso
falls back to **generic behavior**: any event type is accepted, fields are
treated as-is, and the topic is built from sorted keys. This mode is intended
for local development and quick experiments.

Operators may flip the behavior with `notification_schema_strict`:

| `notification_schema` | `notification_schema_strict` | Effective behavior |
|---|---|---|
| non-empty               | `None` (unset)  | **strict** — unknown event types return 400 |
| empty / absent          | `None` (unset)  | permissive generic fallback |
| any                     | `false`         | permissive generic fallback (legacy mode; a startup warning is emitted when the schema is non-empty) |
| any                     | `true`          | strict — unknown event types return 400; with no schema this is deny-all |

Independent of the strict-mode knob, the `event_type` value that ends up on
Prometheus labels and tracing span fields is always bounded: requests whose
`event_type` is not in the configured schema have their observability label
collapsed to the literal `"generic"`. In strict mode this collapsing is rarely
exercised because unknown event_types are already rejected upstream with
`400 UNKNOWN_EVENT_TYPE`; in permissive (legacy generic-fallback) mode it is
the main mechanism preventing unbounded label cardinality from
user-controlled input.

---

## Identifier

An **identifier** is a set of key-value pairs that describe *what* a notification is about.
Think of it as structured metadata that uniquely (or approximately) locates a piece of data.

```json
{
  "identifier": {
    "region":   "north",
    "run_time": "1200",
    "severity": "4",
    "anomaly":  "42.5"
  }
}
```

Identifiers serve two purposes depending on the operation:

| Operation | Role of identifier |
|---|---|
| `notify`  | Declares the metadata of the notification being published |
| `watch`   | Acts as a filter for which notifications to receive |
| `replay`  | Acts as a filter for which historical notifications to retrieve |

In `watch` and `replay`, identifier values can be **constraint objects** instead of scalars
(e.g. `{"gte": 5}`) for numeric and enum fields. See [Streaming Semantics](./streaming-semantics.md).

---

## Topic

A **topic** is the internal routing key Aviso builds from an identifier.
You rarely construct topics manually; Aviso builds them for you.

Topics are dot-separated strings, for example:

```
extreme_event.north.1200.4.42%2E5
```

Each token corresponds to one identifier field, in the order defined by `key_order` in the schema.
The `%2E` is the dot in `42.5`, percent-encoded so it doesn't conflict with the topic separator.
Reserved characters (`.`, `*`, `>`, `%`) in field values are percent-encoded before writing
to the backend so they do not break routing. See [Topic Encoding](./topic-encoding.md).

---

## Schema

A **schema** configures how Aviso handles a specific event type.
Schemas are defined in `configuration/config.yaml` under `notification_schema`.

A schema controls:

- **`topic.base`**: the stream/prefix for this event type (e.g. `diss`, `mars`)
- **`topic.key_order`**: the order of identifier fields in the topic string
- **`identifier.*`**: validation rules per field (type, required, allowed values, ranges)
- **`payload.required`**: whether a payload is mandatory on notify
- **`storage_policy`**: per-stream retention, size limits, compression (JetStream only)

Example:

```yaml
notification_schema:
  extreme_event:
    topic:
      base: "extreme_event"
      key_order: ["region", "run_time", "severity", "anomaly", "polygon"]
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

Every key listed in `key_order` must have a corresponding entry in `identifier`.
For `notify`, every identifier key declared in the schema must be present in the request and every value must pass the handler's validation (an empty string, an out-of-range integer, an unparseable date, and so on are all rejected). The `required` flag has **no effect on notify**; it only changes the behavior of `watch` and `replay`. There, a missing key marked `required: true` returns `400`, while a missing key marked `required: false` is treated as a wildcard. The flag never relaxes value validation; only the presence rules.

---

## Payload

A **payload** is arbitrary JSON attached to a notification.
Aviso treats it as opaque; it stores and replays the value exactly as sent.

Valid payload types: object, array, string, number, boolean, or `null`.

```json
{ "payload": { "path": "/data/grib2/file.grib2", "size": 1048576 } }
```

Whether payload is required or optional is controlled per schema by `payload.required`.
See [Payload Contract](./payload-contract.md) for the full input → storage → replay mapping.

---

## Operations: Notify, Watch, Replay

Aviso exposes three operations, each on its own endpoint:

### Notify: `POST /api/v1/notification`

Publishes a notification to the backend.
The identifier must match all required schema fields exactly (no wildcards, no constraints).

### Watch: `POST /api/v1/watch`

Opens a persistent **Server-Sent Events (SSE)** stream.
Receives live notifications as they arrive, optionally starting from a historical point.

- Omit `from_id`/`from_date` for a live-only stream.
- Provide one of them for historical replay first, then live.

### Replay: `POST /api/v1/replay`

Opens a finite SSE stream of **historical notifications only**.
Requires exactly one of `from_id` or `from_date`.
Stream closes automatically when history is exhausted.

For end-to-end working examples of all three operations, including spatial and constraint filtering, see [Practical Examples](./practical-examples/overview.md).

---

## CloudEvents

Aviso delivers notifications to watch/replay clients as **CloudEvents**, a standard envelope format.
Each event includes:

- `id`: the backend sequence reference (e.g. `mars@42`), used for targeted delete or resume.
- `type`: the Aviso event type string, prefixed with `int.ecmwf.aviso.` (for example `int.ecmwf.aviso.mars`).
- `source`: the server base URL.
- `data.identifier`: the canonicalized identifier.
- `data.payload`: the notification payload (or `null` if omitted).

---

## Backend

The **backend** is the storage and messaging layer that Aviso delegates to.
Two backends are supported:

| Backend | Use case |
|---|---|
| `jetstream` | Production: durable, replicated, persistent history |
| `in_memory` | Development: fast setup, no persistence |

The backend is selected via `notification_backend.kind` in config.
See [Backends Overview](./backends-overview.md).
