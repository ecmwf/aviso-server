# JetStream Backend

## Intended use

`jetstream` backend is the production-oriented backend:

- durable storage,
- replay support,
- live streaming support,
- configurable retention/limits.

## Core behavior

- Connects to configured NATS server.
- Creates streams on demand by topic base.
- Publishes notifications directly to JetStream subjects.
- Uses pull consumers for replay batching.
- Uses consumer-based subscription for live watch streams.

## Local test setup

For local development/testing with Docker:

```bash
./scripts/init_nats.sh
```

Then configure:

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
```

## Stream creation notes

On stream creation, backend applies:

- `storage_type`
- `retention_policy`
- `discard_policy`
- `max_messages`
- `max_bytes`
- `retention_days` -> `max_age`
- `replicas`

Existing streams are reused; configuration is not automatically reconciled for already-created streams.

## Replay behavior

- Sequence replay: start from `from_id`.
- Time replay: start from `from_date` (RFC3339), using JetStream start-time delivery policy.
- API contract still enforces replay parameter exclusivity (`from_id` xor `from_date`).

## Operational caveats

- Core JetStream settings are fail-fast validated before connection and stream operations begin.
- Policy fields (`storage_type`, `retention_policy`, `discard_policy`) are parsed as typed enums during configuration load.
- Stream settings are applied when a stream is created; existing streams are not auto-mutated.
- Startup connectivity is controlled by `timeout_seconds` + `retry_attempts`.
- Runtime reconnect behavior is controlled by `enable_auto_reconnect`, `max_reconnect_attempts`, and `reconnect_delay_ms`.

For detailed field mapping, see [JetStream Settings](./jetstream-settings.md).
