# JetStream Backend

The `jetstream` backend is the production-oriented storage implementation.
It connects to a [NATS](https://nats.io/) server with JetStream enabled and uses it
for durable message storage, replay, and live streaming.

---

## Intended Use

Use `jetstream` when you need:

- durable storage that survives server restarts
- replay across multiple server instances
- live streaming with cluster-wide fan-out
- configurable retention, size limits, and compression

---

## Local Test Setup

Start a NATS + JetStream instance via Docker:

```bash
./scripts/init_nats.sh
```

Then configure Aviso:

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
```

For full setup options including authentication and storage limits, see
[Installation — Local JetStream](./installation.md#local-jetstream-docker).

---

## Core Behavior

- Connects to the configured NATS server on startup (with retry).
- Creates JetStream streams on demand, one per topic `base` (e.g. `MARS`, `DISS`, `POLYGON`).
- Publishes notifications directly to JetStream subjects using the encoded wire format.
- Uses pull consumers for replay batching (`from_id`, `from_date`).
- Uses push consumers for live watch subscriptions.
- Reconciles existing streams against current config when they are first accessed.

---

## Configuration Reference

All fields live under `notification_backend.jetstream`.

### Connection & startup

| Field | Default | Notes |
|---|---|---|
| `nats_url` | `nats://localhost:4222` | NATS server URL. |
| `token` | `None` | Token auth; falls back to `NATS_TOKEN` environment variable. |
| `timeout_seconds` | `30` | Per-attempt connection timeout (`> 0`). |
| `retry_attempts` | `3` | Startup connection attempts before backend init fails (`> 0`). |

### Runtime reconnect

| Field | Default | Notes |
|---|---|---|
| `enable_auto_reconnect` | `true` | Enables/disables NATS client reconnect after startup. |
| `max_reconnect_attempts` | `5` | `0` means unlimited reconnect retries. |
| `reconnect_delay_ms` | `2000` | Delay between reconnect attempts and startup connect retries (`> 0`). |

### Publish resilience

| Field | Default | Notes |
|---|---|---|
| `publish_retry_attempts` | `5` | Retries for transient `channel closed` publish failures (`> 0`). |
| `publish_retry_base_delay_ms` | `150` | Base backoff in ms for publish retries; grows exponentially per attempt (`> 0`). |

### Stream defaults

These apply to every stream created by Aviso unless overridden by a per-schema `storage_policy`.

| Field | Default | Notes |
|---|---|---|
| `max_messages` | `None` | Stream message cap (maps to `max_messages`). |
| `max_bytes` | `None` | Stream size cap in bytes (maps to `max_bytes`). |
| `retention_time` | `None` | Default max age: duration literal (`s`, `m`, `h`, `d`, `w`; e.g. `30d`). |
| `storage_type` | `file` | `file` or `memory` — parsed as typed enum at config load. |
| `replicas` | `None` | Stream replica count. |
| `retention_policy` | `limits` | `limits`, `interest`, or `workqueue` — parsed as typed enum. |
| `discard_policy` | `old` | `old` or `new` — parsed as typed enum. |

> **Fail-fast validation:** `storage_type`, `retention_policy`, and `discard_policy` are parsed
> as typed enums during configuration loading. Invalid values fail startup immediately, before
> any streams are created.

### Full example

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
    timeout_seconds: 30
    retry_attempts: 3
    enable_auto_reconnect: true
    max_reconnect_attempts: 5
    reconnect_delay_ms: 2000
    publish_retry_attempts: 5
    publish_retry_base_delay_ms: 150
    storage_type: file
    retention_policy: limits
    discard_policy: old
```

---

## Stream Management

### Stream creation

On first access (e.g. first publish for a given event type), Aviso creates a JetStream stream
with the following settings applied:

- `storage_type`, `retention_policy`, `discard_policy`
- `max_messages`, `max_bytes`, `retention_time` → `max_age`
- `replicas`

The stream subject binding is set to `<base>.>` (e.g. `mars.>`) to capture all topics
under that base.

### Reconciliation of existing streams

When a stream already exists and is accessed by Aviso, it is **reconciled** — the current
stream config is compared against the desired config and mutable fields are updated if drift
is detected:

- limits (retention, size, message count)
- compression
- duplicate window
- replicas
- subject binding

If JetStream rejects an update (e.g. the field is not editable in the current server/stream state),
Aviso logs a warning and continues with the existing stream configuration.

### Precedence

Backend-level defaults are applied first, then per-schema `storage_policy` overrides for that stream:

```
notification_backend.jetstream.* (base defaults)
    ↓ overridden by
notification_schema.<event_type>.storage_policy.*
```

### Applying config changes to existing streams

Changes to stream-affecting settings (e.g. `compression`, retention, limits) in `config.yaml`
are applied to existing streams automatically during reconciliation when the stream is next accessed.

To force historical data to be physically rewritten with new settings (e.g. re-pack with compression):

1. Stop all Aviso writers for the target stream.
2. Delete the stream in NATS.
3. Restart Aviso (or publish again) — the stream is recreated with current config.

```bash
# List streams
nats stream ls

# Delete a stream (example: DISS)
nats stream rm DISS
```

> `wipe_stream` (admin endpoint) removes messages but preserves stream configuration.
> Use stream deletion only when you need historical data physically rewritten.

---

## Verifying Effective Stream Policy

Use the `nats` CLI to inspect the stream config after a publish or reconcile:

```bash
# Replace POLYGON with your stream name (MARS, DISS, etc.)
nats --server nats://localhost:4222 stream info POLYGON
```

Fields to check:

| CLI field | Config field |
|---|---|
| `Max Age` | `retention_time` |
| `Max Messages` | `max_messages` |
| `Max Bytes` | `max_bytes` / per-schema `max_size` |
| `Max Messages Per Subject` | `allow_duplicates`: `1` = disabled, `-1` = enabled |
| `Compression` | `None` or `S2` |

---

## Replay Behavior

- **Sequence replay** (`from_id`): starts from that sequence number, inclusive.
- **Time replay** (`from_date`): uses JetStream start-time delivery policy.
- The API enforces mutual exclusivity — `from_id` and `from_date` cannot both be present.

---

## Smoke Test (JetStream Mode)

```bash
python3 -m pip install httpx

BACKEND=jetstream \
NATS_URL=nats://localhost:4222 \
JETSTREAM_POLICY_STREAM_NAME=POLYGON \
EXPECT_MAX_MESSAGES=500000 \
EXPECT_MAX_BYTES=2147483648 \
EXPECT_MAX_MESSAGES_PER_SUBJECT=1 \
EXPECT_COMPRESSION=None \
python3 scripts/smoke_test.py
```

---

## Operational Caveats

- Startup connectivity is controlled by `timeout_seconds` + `retry_attempts`.
- Runtime reconnect is controlled by `enable_auto_reconnect`, `max_reconnect_attempts`, `reconnect_delay_ms`.
- Publish retry is a narrow resilience path for transient `channel closed` failures; non-transient failures fail fast.
- `retry_attempts` applies only to startup; post-startup reconnect uses the reconnect settings.
- Setting `max_reconnect_attempts = 0` enables unlimited reconnect retries.
