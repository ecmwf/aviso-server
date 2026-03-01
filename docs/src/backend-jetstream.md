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
- Uses a fixed `.`-separated wire subject format with token encoding for reserved characters.
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
- `retention_time` -> `max_age` (duration literal, for example `7d`, `12h`, `30m`)
- `replicas`

Existing streams are reconciled when that stream is accessed by Aviso (for example, publish path),
including managed subject binding and mutable policy fields (limits/retention/compression/duplicates/replicas).

### Applying config changes to existing streams

Changing stream-affecting settings (for example `compression`, retention, size/message limits)
in `config.yaml` is applied to existing streams during reconciliation.

When a stream is accessed, Aviso compares desired config with current stream config and updates
mutable fields when drift is detected.
If JetStream rejects an update (for example field not editable in current server/stream state),
Aviso logs a warning and continues serving with the existing stream configuration.

To force historical data to reflect new physical layout (for example re-pack all old data with
new compression), recreate the stream:

1. Stop `aviso-server` writers for the target stream.
2. Delete the stream in NATS.
3. Restart the app (or publish again) so the stream is recreated with current config.

Example:

```bash
# list streams
nats stream ls

# remove one stream (example: DISS)
nats stream rm DISS
```

Notes:

- `wipe_stream` removes messages but preserves stream configuration.
- Recreate is only needed when you want historical data physically rewritten.

### Verifying effective stream policy

Use `nats` CLI to inspect the effective stream config after publish/reconcile:

```bash
# Example for test_polygon stream base -> POLYGON stream
nats --server nats://localhost:4222 stream info POLYGON
```

Look at these fields:

- `Max Age` (`retention_time`)
- `Max Messages` (`max_messages`)
- `Max Bytes` (`max_bytes` / per-schema `max_size`)
- `Max Messages Per Subject` (`allow_duplicates`: `1` = disabled, `-1` = enabled)
- `Compression` (`None` or `S2`)

Precedence rule:

- backend defaults (`notification_backend.jetstream.*`) apply first
- per-schema `storage_policy` overrides backend defaults for that stream base

Smoke script tip (JetStream mode):

Make sure the Python dependency is installed first:

```bash
python3 -m pip install httpx
```

```bash
BACKEND=jetstream \
NATS_URL=nats://localhost:4222 \
JETSTREAM_POLICY_STREAM_NAME=POLYGON \
EXPECT_MAX_MESSAGES=500000 \
EXPECT_MAX_BYTES=2147483648 \
EXPECT_MAX_MESSAGES_PER_SUBJECT=1 \
EXPECT_COMPRESSION=None \
python3 scripts/smoke_test.py
```

## Replay behavior

- Sequence replay: start from `from_id`.
- Time replay: start from `from_date` (RFC3339), using JetStream start-time delivery policy.
- API contract still enforces replay parameter exclusivity (`from_id` xor `from_date`).

## Operational caveats

- Core JetStream settings are fail-fast validated before connection and stream operations begin.
- Policy fields (`storage_type`, `retention_policy`, `discard_policy`) are parsed as typed enums during configuration load.
- Stream settings are applied on create and reconciled for existing Aviso-managed streams when accessed.
- Startup connectivity is controlled by `timeout_seconds` + `retry_attempts`.
- Runtime reconnect behavior is controlled by `enable_auto_reconnect`, `max_reconnect_attempts`, and `reconnect_delay_ms`.
- Publish retry behavior for transient transport failures is controlled by
  `publish_retry_attempts` and `publish_retry_base_delay_ms`.

For detailed field mapping, see [JetStream Settings](./jetstream-settings.md).
