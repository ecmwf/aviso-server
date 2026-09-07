# Defining Notification Schemas

A notification schema describes the shape of an event stream: what identifier
fields are accepted, how they are validated, how the storage topic is
constructed, whether a payload is required, and who can read or write.

Schemas are defined under the `notification_schema` key in your configuration
file. Each top-level key becomes an **event type** that clients reference when
calling `/api/v1/notification`, `/api/v1/watch`, or `/api/v1/replay`.

```yaml
notification_schema:
  my_event: # ← event type name
    topic: ...
    identifier: ...
    payload: ...
    auth: ... # optional
    storage_policy: ... # optional, JetStream only
```

---

## Topic Configuration

A schema should have a `topic` block that tells Aviso how to build the NATS
subject for storage and routing.

```yaml
topic:
  base: "weather"
  key_order: ["region", "date"]
```

| Field       | Description                                                                        |
| ----------- | ---------------------------------------------------------------------------------- |
| `base`      | Root prefix for the subject. Must be unique across all schemas (case-insensitive). |
| `key_order` | Ordered list of identifier field names appended to the base, separated by `.`.     |

Given `base: "weather"` and `key_order: ["region", "date"]`, a request with
`region=north` and `date=20250706` produces the subject:

```text
weather.north.20250706
```

Values containing reserved characters (`.`, `*`, `>`, `%`) are automatically
percent-encoded so they do not interfere with NATS subject routing. See
[Topic Encoding](./topic-encoding.md) for details.

When a `topic` block is configured, startup requires a nonempty `key_order`.
Each entry must name a declared identifier and may appear only once. Every
ordinary identifier must be included, even when `required: false`. Optional
fields still need a subject position for watch/replay wildcards and filters.
For example, `key_order: [region, date]` is valid when both fields are declared;
`key_order: [region, region]` is not.

Spatial geometry is the exception. `PolygonHandler` fields may be omitted
because their geometry is stored as metadata. Existing polygon subject
positions remain supported. The reserved `point_cloud` field must never appear
in `key_order`; it uses spatial metadata instead.

There is no request-only authorization exception for ECPDS. Its `match_key`
must appear in a configured topic's `key_order`. Checking permission for a
request value does not restrict delivered events unless routing also retains
that value. Ordinary fields outside `key_order` are not validation-only fields:
their values would be lost from routing and topic-based reconstruction.

These checks apply to configured `topic` blocks. They do not change the generic
fallback used without a topic or schema, including its bare-topic behavior.

---

## Identifier Fields

The `identifier` map defines the fields that clients can send. Each field
specifies a handler type that controls validation and canonicalization.

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

| Property      | Type   | Description                                                                                                                                                                                                           |
| ------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `type`        | string | Handler type (see below). Required.                                                                                                                                                                                   |
| `required`    | bool   | Affects `watch` and `replay` only: if `true`, those requests must include this field; if `false`, missing keys become wildcards. Has **no effect on `notify`**, which always requires every declared field. Required. |
| `description` | string | Human-readable text exposed by `GET /api/v1/schema`. Optional.                                                                                                                                                        |

`PointCloudHandler` is operation-specific. Publishers provide the declared
`point_cloud` field. Watch and replay requests provide a closed `polygon`
instead. That polygon satisfies `required: true`; subscribers must not send
`point_cloud`.

### Handler Types

#### StringHandler

Accepts any non-empty string. No transformation.

```yaml
class:
  type: StringHandler
  max_length: 2 # optional: reject strings longer than this
  required: true
```

#### DateHandler

Parses dates in multiple formats and canonicalizes to a configured output
format.

Accepted inputs: `YYYY-MM-DD`, `YYYYMMDD`, `YYYY-DDD` (day-of-year).

```yaml
date:
  type: DateHandler
  canonical_format: "%Y%m%d" # output format (default: "%Y%m%d")
  required: false
```

| `canonical_format` | Example output |
| ------------------ | -------------- |
| `"%Y%m%d"`         | `20250706`     |
| `"%Y-%m-%d"`       | `2025-07-06`   |

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

Accepts one value from a predefined list. Matching is case-insensitive; stored
in lowercase.

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
  range: [0, 100000] # optional: inclusive [min, max] bounds
  required: false
```

Input `"007"` → stored as `"7"`. Input `"-1"` with `range: [0, 100]` → rejected.

#### FloatHandler

Accepts floating-point strings. Rejects `NaN` and `Inf`.

```yaml
severity:
  type: FloatHandler
  range: [0.0, 10.0] # optional: inclusive [min, max] bounds
  required: false
```

Input `"3.14"` → stored as `"3.14"`. Input `"NaN"` → rejected.

#### ExpverHandler

Experiment version handler. Numeric values are zero-padded to four digits;
non-numeric values are lowercased.

```yaml
expver:
  type: ExpverHandler
  default: "0001" # optional: used when the field is empty
  required: false
```

Input `"1"` → stored as `"0001"`. Input `"test"` → stored as `"test"`.

#### PolygonHandler

Accepts a closed polygon as a JSON array of `[lat,lon]` pairs. Legacy coordinate
strings remain accepted at the HTTP boundary. The first and last pair must be
identical.

Canonical form: `[[lat,lon],...,[lat,lon]]`. Emitted CloudEvents always use this
array form.

```yaml
polygon:
  type: PolygonHandler
  required: true
```

Constraints: at least three coordinate pairs plus the closing repeat. Latitude
must be in `[-90, 90]`; longitude must be in `[-180, 180]`.

#### PointCloudHandler

Spatial identifiers use fixed names: `PolygonHandler` must be named `polygon`,
and `PointCloudHandler` must be named `point_cloud`. A schema cannot mix the two
handlers or declare multiple geometries. Polygon metadata and existing routed
polygon subjects are supported.

Accepts a non-empty JSON array of `[lat,lon]` pairs from providers. Point clouds
do not have a string syntax and do not need a closing point. Duplicates are
valid. Their order is preserved.

```yaml
point_cloud:
  type: PointCloudHandler
  required: true
  max_points: 10000
  description: >-
    Publishers provide point_cloud as [[latitude, longitude], ...]. Watch and
    replay requests provide a closed polygon instead. The polygon satisfies
    this required field; subscribers must not send point_cloud.
```

`max_points` defaults to 10,000 and cannot exceed 10,000. Canonical point-cloud
JSON is also limited to 60 KiB. This is a conservative Aviso interoperability
limit informed by NATS-backed header transport and near-limit round-trip tests.
It leaves room for Aviso's other metadata, but is not a protocol-wide NATS
header limit. The point-count check runs first.

The handler must use the reserved `point_cloud` identifier key. Only one is
allowed per schema, and the schema must define a `topic` block. Do not put
`point_cloud` in `topic.key_order`; startup rejects it. Aviso stores the cloud
in spatial metadata instead of the subject.

Point-cloud subscribers use `polygon` on `/watch` and `/replay`. A valid polygon
satisfies a required `point_cloud` field for those operations. A schema with a
`PointCloudHandler` cannot declare `polygon`, since that key is reserved for the
query-time filter.

See [Spatial Filtering](./practical-examples/spatial-filtering.md) for usage
examples.

### Reserved Query-Time Fields

#### `point` (built-in)

The `point` field is a reserved identifier that clients can send on `/watch` or
`/replay` to filter notifications whose polygon contains the point. Canonical
form is `[lat,lon]`; compatible `lat,lon` strings are also accepted.

`point` is **not** a schema-configurable handler. It is available on schemas
that include a `PolygonHandler`. The `/notification` endpoint rejects it.

See [Spatial Filtering](./practical-examples/spatial-filtering.md) for usage
examples.

---

## Payload Configuration

Controls whether requests must include a `payload` field.

```yaml
payload:
  required: true
```

| `required` | Behavior                                                         |
| ---------- | ---------------------------------------------------------------- |
| `true`     | Requests without a payload are rejected (400).                   |
| `false`    | Payload is optional; missing payloads are stored as JSON `null`. |

The payload can be any valid JSON value (object, array, string, number, boolean,
null). It is stored as-is with no reshaping.

See [Payload Contract](./payload-contract.md) for full semantics.

---

## Per-Stream Authentication

When [global authentication](./authentication.md) is enabled, individual schemas
can require credentials and restrict access by role.

```yaml
auth:
  required: true
  read_roles:
    localrealm: ["analyst", "consumer"]
  write_roles:
    localrealm: ["producer"]
```

| Field         | Default when omitted            | Effect                                          |
| ------------- | ------------------------------- | ----------------------------------------------- |
| `required`    | (none)                          | Must be set explicitly to `true` or `false`.    |
| `read_roles`  | Any authenticated user can read | Maps realm → role list for watch/replay access. |
| `write_roles` | Only admins can write           | Maps realm → role list for notify access.       |

Use `["*"]` as the role list to grant access to all users from a realm.

Admins (users matching global `admin_roles`) always have both read and write
access.

Omitting the entire `auth` block makes the stream publicly accessible, even when
global auth is enabled.

See [Authentication](./authentication.md) for the full access-control matrix and
role-matching rules.

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

| Field              | Type     | Description                                                        |
| ------------------ | -------- | ------------------------------------------------------------------ |
| `retention_time`   | duration | Discard messages older than this. Accepts `30m`, `1h`, `7d`, `1w`. |
| `max_messages`     | integer  | Maximum message count; oldest are discarded when exceeded.         |
| `max_size`         | size     | Maximum stream size. Accepts `100Mi`, `1Gi`, etc.                  |
| `allow_duplicates` | bool     | Allow duplicate message IDs. Default: backend-specific.            |
| `compression`      | bool     | Enable message-level compression. Default: backend-specific.       |

All fields are optional. Omitting `storage_policy` entirely uses backend
defaults.

The in-memory backend does not support storage policies.

---

## Complete Example

This example defines a weather alert stream with date/region routing, enum
validation, optional payload, and role-restricted access.

```yaml
notification_schema:
  weather_alert:
    payload:
      required: false

    topic:
      base: "alert"
      key_order: ["region", "severity_level", "date", "issued_by"]

    identifier:
      region:
        description: "Geographic region."
        type: EnumHandler
        values: ["europe", "asia", "africa", "americas", "oceania"]
        required: true
      severity_level:
        description: "Alert severity (1 to 5)."
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

- Publishing a notification with `region=europe`, `severity_level=3`,
  `date=2025-07-06`, `issued_by=forecast` produces the subject
  `alert.europe.3.20250706.forecast`.
- Publishers must provide `issued_by`. Watch/replay clients may omit it to
  match any issuer because it is declared `required: false`.
- Any authenticated user in the `operations` realm can watch/replay.
- Only users with the `forecaster` or `admin` role can publish.
- JetStream retains up to 100,000 messages or 30 days, whichever limit is hit
  first.

---

## Tips

- **Start simple.** Define only `topic`, one or two `identifier` fields, and
  `payload`. Add auth and storage policy later.
- **Use `key_order` deliberately.** Fields in `key_order` become part of the
  NATS subject and affect routing granularity. More fields = more specific
  topics = more efficient filtering, but also more distinct subjects.
- **Choose subscriber requirements.** Set `required: true` when watch/replay
  clients must supply a field. Publishers always supply every declared field.
- **Keep `base` short and unique.** It is the root of every subject in this
  stream. Avoid collisions with other schemas.
- **Test with `GET /api/v1/schema/{event_type}`.** This endpoint returns the
  public view of your schema, showing all identifier fields and their validation
  rules.
