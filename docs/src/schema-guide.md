# Defining Notification Schemas

A notification schema describes the shape of an event stream: what identifier fields are accepted, how they are validated, how the storage topic is constructed, whether a payload is required, and who can read or write.

Schemas are defined under the `notification_schema` key in your configuration file. Each top-level key becomes an **event type** that clients reference when calling `/api/v1/notification`, `/api/v1/watch`, or `/api/v1/replay`.

```yaml
notification_schema:
  my_event:          # ← event type name
    topic: ...
    identifier: ...
    payload: ...
    auth: ...            # optional
    storage_policy: ...  # optional, JetStream only
```

---

## Topic Configuration

A schema should have a `topic` block that tells Aviso how to build the NATS subject for storage and routing.

```yaml
topic:
  base: "weather"
  key_order: ["region", "date"]
```

| Field | Description |
|---|---|
| `base` | Root prefix for the subject. Must be unique across all schemas (case-insensitive). |
| `key_order` | Ordered list of identifier field names appended to the base, separated by `.`. |

Given `base: "weather"` and `key_order: ["region", "date"]`, a request with `region=north` and `date=20250706` produces the subject:

```
weather.north.20250706
```

Values containing reserved characters (`.`, `*`, `>`, `%`) are automatically percent-encoded so they do not interfere with NATS subject routing. See [Topic Encoding](./topic-encoding.md) for details.

Only fields listed in `key_order` contribute to the subject. Other identifier fields are validated but not part of the topic.

---

## Identifier Fields

The `identifier` map defines the fields that clients can send. Each field specifies a handler type that controls validation and canonicalization.

```yaml
identifier:
  region:
    type: EnumHandler
    values: ["north", "south", "east", "west"]
    required: true
    description: "Geographic region."
  date:
    type: DateHandler
    required: true
```

Every field supports these common properties:

| Property | Type | Description |
|---|---|---|
| `type` | string | Handler type (see below). Required. |
| `required` | bool | If `true`, requests missing this field are rejected. Required. |
| `description` | string | Human-readable text exposed by `GET /api/v1/schema`. Optional. |

### Handler Types

#### StringHandler

Accepts any non-empty string. No transformation.

```yaml
class:
  type: StringHandler
  max_length: 2      # optional: reject strings longer than this
  required: true
```

#### DateHandler

Parses dates in multiple formats and canonicalizes to a configured output format.

Accepted inputs: `YYYY-MM-DD`, `YYYYMMDD`, `YYYY-DDD` (day-of-year).

```yaml
date:
  type: DateHandler
  canonical_format: "%Y%m%d"   # output format (default: "%Y%m%d")
  required: false
```

| `canonical_format` | Example output |
|---|---|
| `"%Y%m%d"` | `20250706` |
| `"%Y-%m-%d"` | `2025-07-06` |

Invalid dates (e.g. February 30) are rejected.

#### TimeHandler

Parses times and canonicalizes to four-digit `HHMM` format.

Accepted inputs: `14:30`, `1430`, `14`, `9:05`.

```yaml
time:
  type: TimeHandler
  required: false
```

Input `14:30` → stored as `1430`. Input `9` → stored as `0900`.

#### EnumHandler

Accepts one value from a predefined list. Matching is case-insensitive; stored in lowercase.

```yaml
domain:
  type: EnumHandler
  values: ["a", "b", "c"]
  required: false
```

Input `"A"` → stored as `"a"`. Input `"x"` → rejected.

#### IntHandler

Accepts integer strings. Strips leading zeros for canonical storage.

```yaml
step:
  type: IntHandler
  range: [0, 100000]   # optional: inclusive [min, max] bounds
  required: false
```

Input `"007"` → stored as `"7"`. Input `"-1"` with `range: [0, 100]` → rejected.

#### FloatHandler

Accepts floating-point strings. Rejects `NaN` and `Inf`.

```yaml
severity:
  type: FloatHandler
  range: [0.0, 10.0]   # optional: inclusive [min, max] bounds
  required: false
```

Input `"3.14"` → stored as `"3.14"`. Input `"NaN"` → rejected.

#### ExpverHandler

Experiment version handler. Numeric values are zero-padded to four digits; non-numeric values are lowercased.

```yaml
expver:
  type: ExpverHandler
  default: "0001"    # optional: used when the field is empty
  required: false
```

Input `"1"` → stored as `"0001"`. Input `"test"` → stored as `"test"`.

#### PolygonHandler

Accepts a closed polygon as a coordinate string. Used for spatial filtering on `/watch` and `/replay`.

Format: `lat,lon,lat,lon,...,lat,lon` — the first and last coordinate pair must be identical to close the polygon. Parentheses are optional.

```yaml
polygon:
  type: PolygonHandler
  required: true
```

Constraints: at least 3 coordinate pairs (plus closing repeat), latitude in [-90, 90], longitude in [-180, 180].

See [Spatial Filtering](./practical-examples/spatial-filtering.md) for usage examples.

#### PointHandler

Accepts a single `lat,lon` coordinate pair. This is a query-time field: clients send it on `/watch` or `/replay` to filter notifications whose polygon contains the point. The `/notification` endpoint rejects requests that include a `point` field.

```yaml
location:
  type: PointHandler
  required: false
```

---

## Payload Configuration

Controls whether requests must include a `payload` field.

```yaml
payload:
  required: true
```

| `required` | Behavior |
|---|---|
| `true` | Requests without a payload are rejected (400). |
| `false` | Payload is optional; missing payloads are stored as JSON `null`. |

The payload can be any valid JSON value (object, array, string, number, boolean, null). It is stored as-is with no reshaping.

See [Payload Contract](./payload-contract.md) for full semantics.

---

## Per-Stream Authentication

When [global authentication](./authentication.md) is enabled, individual schemas can require credentials and restrict access by role.

```yaml
auth:
  required: true
  read_roles:
    localrealm: ["analyst", "consumer"]
  write_roles:
    localrealm: ["producer"]
```

| Field | Default when omitted | Effect |
|---|---|---|
| `required` | — | Must be set explicitly to `true` or `false`. |
| `read_roles` | Any authenticated user can read | Maps realm → role list for watch/replay access. |
| `write_roles` | Only admins can write | Maps realm → role list for notify access. |

Use `["*"]` as the role list to grant access to all users from a realm.

Admins (users matching global `admin_roles`) always have both read and write access.

Omitting the entire `auth` block makes the stream publicly accessible, even when global auth is enabled.

See [Authentication](./authentication.md) for the full access-control matrix and role-matching rules.

---

## Storage Policy (JetStream Only)

When using the JetStream backend, you can configure per-stream retention limits.

```yaml
storage_policy:
  retention_time: "7d"
  max_messages: 500000
  max_size: "2Gi"
  allow_duplicates: false
  compression: true
```

| Field | Type | Description |
|---|---|---|
| `retention_time` | duration | Discard messages older than this. Accepts `30m`, `1h`, `7d`, `1w`. |
| `max_messages` | integer | Maximum message count; oldest are discarded when exceeded. |
| `max_size` | size | Maximum stream size. Accepts `100Mi`, `1Gi`, etc. |
| `allow_duplicates` | bool | Allow duplicate message IDs. Default: backend-specific. |
| `compression` | bool | Enable message-level compression. Default: backend-specific. |

All fields are optional. Omitting `storage_policy` entirely uses backend defaults.

The in-memory backend does not support storage policies.

---

## Complete Example

This example defines a weather alert stream with date/region routing, enum validation, optional payload, and role-restricted access.

```yaml
notification_schema:
  weather_alert:
    payload:
      required: false

    topic:
      base: "alert"
      key_order: ["region", "severity_level", "date"]

    identifier:
      region:
        description: "Geographic region."
        type: EnumHandler
        values: ["europe", "asia", "africa", "americas", "oceania"]
        required: true
      severity_level:
        description: "Alert severity (1–5)."
        type: IntHandler
        range: [1, 5]
        required: true
      date:
        description: "Alert date."
        type: DateHandler
        canonical_format: "%Y%m%d"
        required: true
      issued_by:
        description: "Issuing authority identifier."
        type: StringHandler
        max_length: 64
        required: false

    auth:
      required: true
      read_roles:
        operations: ["*"]
      write_roles:
        operations: ["forecaster", "admin"]

    storage_policy:
      retention_time: "30d"
      max_messages: 100000
```

With this schema:

- Publishing a notification with `region=europe`, `severity_level=3`, `date=2025-07-06` produces the subject `alert.europe.3.20250706`.
- The `issued_by` field is validated if present but does not appear in the subject (not in `key_order`).
- Any authenticated user in the `operations` realm can watch/replay.
- Only users with the `forecaster` or `admin` role can publish.
- JetStream retains up to 100,000 messages or 30 days, whichever limit is hit first.

---

## Tips

- **Start simple.** Define only `topic`, one or two `identifier` fields, and `payload`. Add auth and storage policy later.
- **Use `key_order` deliberately.** Fields in `key_order` become part of the NATS subject and affect routing granularity. More fields = more specific topics = more efficient filtering, but also more distinct subjects.
- **Mark routing fields required.** If a field is in `key_order`, consider making it `required: true` so every notification produces a complete subject.
- **Keep `base` short and unique.** It is the root of every subject in this stream. Avoid collisions with other schemas.
- **Test with `GET /api/v1/schema/{event_type}`.** This endpoint returns the public view of your schema, showing all identifier fields and their validation rules.
