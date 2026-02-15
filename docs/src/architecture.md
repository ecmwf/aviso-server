# Architecture

High-level flow:

1. HTTP routes accept requests for `notification`, `watch`, and `replay`.
2. Request handlers validate fields against schema and normalize parameters.
3. Notification backend abstraction dispatches to an implementation (`jetstream` or `in_memory`).
4. SSE layer emits replay and/or live events.

## Main components

- `src/routes/*`: endpoint entry points.
- `src/handlers/*`: shared request parsing/validation/processing.
- `src/notification/*`: schema-aware topic building and filtering.
- `src/notification_backend/*`: backend abstraction and JetStream/InMemory implementations.
- `src/sse/*`: stream composition, heartbeats, control events, graceful shutdown.

## Backend capability snapshot

| Capability | JetStream | InMemory |
|---|---|---|
| Durable storage | Yes | No |
| Replay support | Yes | Not implemented |
| Live watch support | Yes | Not implemented |
| Multi-replica suitability | Yes (with clustered NATS) | No |

## JetStream backend path

- Config decode: `configuration::JetStreamSettings`
- Internal config: `notification_backend::jetstream::JetStreamConfig`
- Connect: `notification_backend::jetstream::connection`
- Stream setup: `notification_backend::jetstream::streams`
- Publish: `notification_backend::jetstream::publisher`
- Replay: `notification_backend::jetstream::replay`
- Subscribe: `notification_backend::jetstream::subscriber`
