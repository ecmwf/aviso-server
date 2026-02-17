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

## Current implementation status

The following backend methods are not implemented (`todo!`) in current code:

- `subscribe_to_topic`
- `get_messages_batch`

Operational impact:

- `watch` and `replay` endpoints are not supported with `in_memory` backend.
- `notification` endpoint works for transient storage/testing.

## Configuration

`notification_backend.kind: in_memory`

Available knobs:

- `max_history_per_topic` (default `1`)
- `max_topics` (default `10000`)
- `enable_metrics` (default `false`)

## Production suitability

Not recommended for production because:

- no durability,
- no HA replication,
- no cross-instance consistency,
- streaming support is currently incomplete.

