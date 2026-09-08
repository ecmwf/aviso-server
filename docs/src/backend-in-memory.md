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

Historical delivery uses the same request-wide
`watch_endpoint.max_historical_notifications` cap as JetStream, after request
filtering and successful rendering. A schema's `max_historical_notifications`
can override the global cap. This is separate from retention and batch size.
See
[Historical Replay Limits](./streaming-semantics.md#historical-replay-limits)
for truncation controls and watch behavior.

Watch creates its broadcast receiver and captures the last allocated sequence
under the same lock used to store and publish notifications. Replay reads only
up to that inclusive bound; live delivery starts above it. Replay-only captures
the bound under that lock without creating a receiver. Eviction or deletion can
still remove history during replay, and a lagging live receiver can lose queued
notifications. The bound fixes the sequence range, not the stored contents.

## Configuration

`notification_backend.kind: in_memory`

Available knobs:

- `max_history_per_topic` (default `1`)
- `max_topics` (default `10000`)
- `enable_metrics` (default `false`)

Per-schema `storage_policy` fields are currently not supported on `in_memory`
and are rejected at startup.

## Production suitability

Not recommended for production because:

- no durability,
- no HA replication,
- no cross-instance consistency,
- replay/watch history is limited to local in-memory retention.
