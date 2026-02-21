# InMemory Backend

## Intended use

`in_memory` backend is best for:

- local development,
- schema/validation testing,
- lightweight experimentation where persistence is not required.

## Behavior

- Data is process-memory only and is lost on restart.
- Topic/message limits are enforced with eviction.
- No shared state across replicas or pods.
- Supports live watch subscriptions (live-only delivery).
- Supports replay batch retrieval for `from_id` and `from_date`.
- Uses in-process fanout only, so subscriptions/replay are node-local.

## Configuration

`notification_backend.kind: in_memory`

Available knobs:

- `max_history_per_topic` (default `1`)
- `max_topics` (default `10000`)
- `enable_metrics` (default `false`)

Per-schema `storage_policy` fields are currently not supported on `in_memory` and are rejected at startup.

## Production suitability

Not recommended for production because:

- no durability,
- no HA replication,
- no cross-instance consistency,
- replay/watch history is limited to local in-memory retention.
