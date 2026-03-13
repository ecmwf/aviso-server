# Architecture

Aviso Server is built around three operations — **Notify**, **Watch**, and **Replay** —
each sharing a common validation and schema layer but diverging at the backend interaction.

---

## System Overview

```mermaid
graph TB
    subgraph Clients
        P(Publisher)
        W(Watcher)
        R(Replayer)
    end

    subgraph "Aviso Server"
        direction TB
        AM["Auth Middleware<br/>(optional)"]
        RT["Routes<br/>HTTP handlers"]
        VP["Validation &<br/>Processing"]
        NC["Notification<br/>Core"]
        BE["Backend<br/>Abstraction"]
    end

    AOT["auth-o-tron<br/>(external)"]

    subgraph Backend
        JS[("JetStream<br/>NATS")]
        IM[("In-Memory<br/>Process")]
    end

    P -->|POST /api/v1/notification| AM
    W -->|POST /api/v1/watch| AM
    R -->|POST /api/v1/replay| AM

    AM -.->|verify credentials| AOT
    AM --> RT
    RT --> VP
    VP --> NC
    NC --> BE
    BE --> JS
    BE --> IM

    JS -.->|SSE stream| W
    JS -.->|SSE stream| R
    IM -.->|SSE stream| W
    IM -.->|SSE stream| R
```

---

## Notify Request Flow

When a publisher sends `POST /api/v1/notification`:

```mermaid
sequenceDiagram
    participant C as Publisher
    participant A as Auth Middleware
    participant R as Route Handler
    participant V as Validator
    participant P as Processor
    participant T as Topic Builder
    participant B as Backend

    C->>A: POST /api/v1/notification (JSON)
    alt stream requires auth
        A->>A: resolve user (JWT or auth-o-tron)
        A-->>C: 401/403 if unauthorized
    end
    A->>R: forward request (+ user identity)
    R->>V: parse & shape-check JSON
    V-->>R: 400 if malformed
    R->>P: process_notification_request()
    P->>P: look up event schema
    P->>P: validate each identifier field
    P->>P: canonicalize values (dates, enums)
    P->>T: build_topic_with_schema()
    T-->>P: topic string (e.g. mars.od.0001.g.20250706.1200)
    P->>B: put_message_with_headers()
    B-->>C: 200 { id, topic }
```

Key steps:

1. **Parse** — raw JSON bytes are deserialized; unknown fields are rejected (`UNKNOWN_FIELD`)
2. **Validate** — each identifier field is checked against its `ValidationRules` (type, range, enum values)
3. **Canonicalize** — values are normalized (e.g. dates to `YYYYMMDD`, enums to lowercase)
4. **Build topic** — fields are ordered per `key_order`, reserved chars are percent-encoded
5. **Store** — the message is written to the backend with the encoded topic as the subject

---

## Watch Request Flow

`POST /api/v1/watch` opens a persistent SSE stream. It optionally starts with a historical
replay phase before transitioning to live delivery.

```mermaid
sequenceDiagram
    participant C as Subscriber
    participant A as Auth Middleware
    participant R as Route Handler
    participant P as Stream Processor
    participant F as Hybrid Filter
    participant B as Backend

    C->>A: POST /api/v1/watch (JSON)
    alt stream requires auth
        A->>A: resolve user (JWT or auth-o-tron)
        A-->>C: 401/403 if unauthorized
    end
    A->>R: forward request (+ user identity)
    R->>P: process_request (ValidationConfig::for_watch)
    P->>P: allow optional fields & constraint objects
    P->>P: analyze_watch_pattern() → coarse + precise patterns

    alt has from_id or from_date
        P->>B: fetch historical batch
        B-->>P: NotificationMessage[]
        P->>F: apply wildcard + constraint + spatial filter
        F-->>C: SSE: replay_started → events → replay_completed
    end

    P->>B: subscribe(coarse_pattern)
    loop live stream
        B-->>P: live NotificationMessage
        P->>F: apply precise filter
        F-->>C: SSE: notification event
    end

    C-->>R: disconnect / timeout
    R-->>C: SSE: connection_closing
```

---

## Replay Request Flow

`POST /api/v1/replay` is like watch but historical-only — the stream closes when history ends.

```mermaid
sequenceDiagram
    participant C as Client
    participant A as Auth Middleware
    participant R as Route Handler
    participant P as Stream Processor
    participant B as Backend

    C->>A: POST /api/v1/replay (JSON + from_id or from_date)
    alt stream requires auth
        A->>A: resolve user (JWT or auth-o-tron)
        A-->>C: 401/403 if unauthorized
    end
    A->>R: forward request (+ user identity)
    R->>P: process_request (ValidationConfig::for_replay)
    P->>B: batch fetch from StartAt::Sequence or StartAt::Date
    loop batches
        B-->>P: NotificationMessage[]
        P->>P: filter + convert to CloudEvent
        P-->>C: SSE: notification events
    end
    P-->>C: SSE: replay_completed → connection_closing (end_of_stream)
```

---

## SSE Streaming Pipeline

The streaming layer (`src/sse/`) is built around typed values rather than raw strings,
which keeps the lifecycle explicit and the endpoint logic thin.

**Cursor types** — how a start point is represented internally:

- `StartAt::LiveOnly` — no history, subscribe immediately
- `StartAt::Sequence(u64)` — start from a specific backend sequence number
- `StartAt::Date(DateTime<Utc>)` — start from a UTC timestamp

**Frame types** — what the stream produces before rendering to SSE wire format:

- Control frames — `connection_established`, `replay_started`, `replay_completed`
- Notification frames — a decoded CloudEvent ready for delivery
- Heartbeat frames — periodic keep-alive
- Error frames — non-fatal stream errors
- Close frame — carries one of: `end_of_stream`, `max_duration_reached`, `server_shutdown`

Lifecycle (shutdown token, max duration, natural end) is applied once in a shared wrapper,
so individual endpoint handlers don't need to reimplement it.

---

## Component Map

| Component | Path | Role |
|---|---|---|
| Routes | `src/routes/` | Thin HTTP handlers — parse request, delegate, return response |
| Auth | `src/auth/` | Middleware, JWT validation, role matching, auth-o-tron client |
| Handlers | `src/handlers/` | Shared parsing, validation, and processing logic |
| Notification core | `src/notification/` | Schema registry, topic builder/codec/parser, wildcard matcher, spatial |
| Backend abstraction | `src/notification_backend/` | `NotificationBackend` trait + JetStream and InMemory implementations |
| SSE layer | `src/sse/` | Stream composition, typed frames, heartbeats, lifecycle |
| CloudEvents | `src/cloudevents/` | Converts stored messages into CloudEvent envelope |
| Configuration | `src/configuration/` | Config loading, schema validation, global snapshot |
| Error model | `src/error.rs` | Stable HTTP error codes and structured responses |

---

## Hybrid Filtering

Watch subscriptions use a two-tier strategy to balance backend load with filter precision:

```mermaid
graph LR
    A[Watch Request] --> B[analyze_watch_pattern]
    B --> C["Coarse pattern<br/>e.g. mars.*.*.*"]
    B --> D["Precise pattern<br/>full decoded topic"]
    C -->|backend subscription| E[(NATS JetStream)]
    E -->|candidate messages| F[App-level filter]
    D --> F
    F -->|matched| G[SSE client]
    F -->|rejected| H[dropped]
```

- The **coarse pattern** is sent to the backend as the subscription subject filter.
  It uses NATS wildcards and covers a superset of the desired messages.
- The **precise pattern** is applied in-process on decoded topics + constraint objects + spatial checks.
  Only messages that pass both layers reach the client.

This avoids creating one backend subscription per unique topic while still delivering exact results.

---

## JetStream Backend Internals

| Module | Path | Responsibility |
|---|---|---|
| Config | `notification_backend/jetstream/config.rs` | Decode and validate JetStream settings |
| Connection | `notification_backend/jetstream/connection.rs` | NATS connect with retry |
| Streams | `notification_backend/jetstream/streams.rs` | Create and reconcile streams |
| Publisher | `notification_backend/jetstream/publisher.rs` | Publish with retry on transient failures |
| Subscriber | `notification_backend/jetstream/subscriber.rs` | Consumer-based live subscriptions |
| Replay | `notification_backend/jetstream/replay.rs` | Pull consumer batch retrieval |
| Admin | `notification_backend/jetstream/admin.rs` | Wipe and delete operations |
