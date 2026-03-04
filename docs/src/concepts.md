# Key Concepts

This page defines the core terms you will encounter throughout Aviso's documentation and API.
Reading it before Getting Started will make everything else click faster.

---

## Event Type

An **event type** is a named category of notification — for example `extreme_event`, `sensor_alert`, or `data_ready`.

Every request to Aviso (notify, watch, or replay) targets exactly one event type.
The server uses the event type to:

- look up the matching schema (validation rules, required fields, topic ordering),
- route messages to the correct storage stream,
- apply the correct retention and storage policy.

```json
{ "event_type": "extreme_event", ... }
```

If no schema is configured for an event type, Aviso falls back to generic behavior:
fields are accepted as-is and the topic is built from sorted keys.

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
| `watch`   | Acts as a filter — which notifications to receive |
| `replay`  | Acts as a filter — which historical notifications to retrieve |

In `watch` and `replay`, identifier values can be **constraint objects** instead of scalars
(e.g. `{"gte": 5}`) for numeric and enum fields. See [Streaming Semantics](./streaming-semantics.md).

---

## Topic

A **topic** is the internal routing key Aviso builds from an identifier.
You rarely construct topics manually — Aviso builds them for you.

Topics are dot-separated strings, for example:

```
extreme_event.north.1200.4.42%2E5
```

Each token corresponds to one identifier field, in the order defined by `key_order` in the schema.
Note the `%2E` — the dot in `42.5` is percent-encoded so it doesn't conflict with the topic separator.
Reserved characters (`.`, `*`, `>`, `%`) in field values are percent-encoded before writing
to the backend so they do not break routing. See [Topic Encoding](./topic-encoding.md).

---

## Schema

A **schema** configures how Aviso handles a specific event type.
Schemas are defined in `configuration/config.yaml` under `notification_schema`.

A schema controls:

- **`topic.base`** — the stream/prefix for this event type (e.g. `diss`, `mars`)
- **`topic.key_order`** — the order of identifier fields in the topic string
- **`identifier.*`** — validation rules per field (type, required, allowed values, ranges)
- **`payload.required`** — whether a payload is mandatory on notify
- **`storage_policy`** — per-stream retention, size limits, compression (JetStream only)

Example:

```yaml
notification_schema:
  extreme_event:
    topic:
      base: "extreme_event"
      key_order: ["region", "run_time", "severity", "anomaly", "polygon"]
    identifier:
      region:
        - type: EnumHandler
          values: ["north", "south", "east", "west"]
          required: true
      run_time:
        - type: TimeHandler
          required: true
      severity:
        - type: IntHandler
          range: [1, 7]
          required: true
      anomaly:
        - type: FloatHandler
          range: [0.0, 100.0]
          required: false
      polygon:
        - type: PolygonHandler
          required: false
    payload:
      required: false
```

Every key listed in `key_order` must have a corresponding entry in `identifier`.
Optional fields (e.g. `anomaly`, `polygon`) may be omitted from notify requests
but must still be declared in the schema so Aviso knows their type and position in the topic.

---

## Payload

A **payload** is arbitrary JSON attached to a notification.
Aviso treats it as opaque — it stores and replays the value exactly as sent.

Valid payload types: object, array, string, number, boolean, or `null`.

```json
{ "payload": { "path": "/data/grib2/file.grib2", "size": 1048576 } }
```

Whether payload is required or optional is controlled per schema by `payload.required`.
See [Payload Contract](./payload-contract.md) for the full input → storage → replay mapping.

---

## Operations: Notify, Watch, Replay

Aviso exposes three operations, each on its own endpoint:

### Notify — `POST /api/v1/notification`

Publishes a notification to the backend.
The identifier must match all required schema fields exactly (no wildcards, no constraints).

### Watch — `POST /api/v1/watch`

Opens a persistent **Server-Sent Events (SSE)** stream.
Receives live notifications as they arrive, optionally starting from a historical point.

- Omit `from_id`/`from_date` → live-only stream
- Provide one of them → historical replay first, then live

### Replay — `POST /api/v1/replay`

Opens a finite SSE stream of **historical notifications only**.
Requires exactly one of `from_id` or `from_date`.
Stream closes automatically when history is exhausted.

For end-to-end working examples of all three operations — including spatial and constraint filtering — see [Practical Examples](./practical-examples/overview.md).

---

## CloudEvents

Aviso delivers notifications to watch/replay clients as **CloudEvents** — a standard envelope format.
Each event includes:

- `id` — the backend sequence reference (e.g. `mars@42`), used for targeted delete or resume
- `type` — the Aviso event type string
- `source` — the server base URL
- `data.identifier` — the canonicalized identifier
- `data.payload` — the notification payload (or `null` if omitted)

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
