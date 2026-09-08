# JetStream Backend

The `jetstream` backend is the production-oriented storage implementation. It
connects to a [NATS](https://nats.io/) server with JetStream enabled and uses it
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
[Installation: Local JetStream](./installation.md#local-jetstream-docker).

---

## Core Behavior

- Connects to the configured NATS server on startup (with retry).
- Creates JetStream streams on demand, one per topic `base` (e.g. `MARS`,
  `DISS`, `POLYGON`).
- Publishes notifications directly to JetStream subjects using the encoded wire
  format.
- Uses pull consumers for replay batching (`from_id`, `from_date`).
- Uses push consumers for live watch subscriptions.
- Reconciles existing streams against current config when they are first
  accessed.

Pull batches use `watch_endpoint.replay_batch_size` independently of the
request-wide `max_historical_notifications` delivery cap. The shared SSE layer
applies that cap after request filtering and successful rendering, not inside
each backend batch. A schema's `max_historical_notifications` can override the
global cap independently of its storage policy.
Watch captures its history bound from the initial `DeliverNew` consumer create
response's `delivered.stream_sequence`, before any pulls or consumer-info
refresh. The consumer and bound therefore share one creation point, including
when the tail was deleted or the stream is empty at a nonzero sequence. A retry
uses the final successful consumer's bound. Replay-only reads the stream's last
sequence during setup without creating a live consumer. Neither operation
freezes retention or deletion.

See [Historical Replay Limits](./streaming-semantics.md#historical-replay-limits)
for truncation controls and watch behavior.

---

## Configuration Reference

All fields live under `notification_backend.jetstream`.

### Connection & startup

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`nats_url`](#notification-backend-jetstream-nats-url) | `nats://localhost:4222` |
| [`token`](#notification-backend-jetstream-token) | `None` |
| [`timeout_seconds`](#notification-backend-jetstream-timeout-seconds) | `30` |
| [`retry_attempts`](#notification-backend-jetstream-retry-attempts) | `3` |

</div>
<details class="setting-panel" id="notification-backend-jetstream-nats-url">
<summary><code>nats_url</code>
<span class="setting-meta"><strong>Default:</strong> <code>nats://localhost:4222</code></span>
</summary>

NATS server URL.

</details>
<details class="setting-panel" id="notification-backend-jetstream-token">
<summary><code>token</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code></span>
</summary>

Token auth; falls back to `NATS_TOKEN` environment variable.

</details>
<details class="setting-panel" id="notification-backend-jetstream-timeout-seconds">
<summary><code>timeout_seconds</code>
<span class="setting-meta"><strong>Default:</strong> <code>30</code></span>
</summary>

Per-attempt connection timeout (`> 0`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-retry-attempts">
<summary><code>retry_attempts</code>
<span class="setting-meta"><strong>Default:</strong> <code>3</code></span>
</summary>

Startup connection attempts before backend init fails (`> 0`).

</details>
</div>

### Runtime reconnect

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`max_reconnect_attempts`](#notification-backend-jetstream-max-reconnect-attempts) | unlimited |
| [`reconnect_delay_ms`](#notification-backend-jetstream-reconnect-delay-ms) | `2000` |

</div>
<details class="setting-panel" id="notification-backend-jetstream-max-reconnect-attempts">
<summary><code>max_reconnect_attempts</code>
<span class="setting-meta"><strong>Default:</strong> unlimited</span>
</summary>

Unset and `0` both mean unlimited reconnect retries; set a positive value only
if you explicitly want the client to give up (the backend then stays
disconnected until a process restart).

Subscription creation uses a bounded retry loop: unset means five attempts,
`0` means one attempt, and a positive value sets the attempt limit.

</details>
<details class="setting-panel" id="notification-backend-jetstream-reconnect-delay-ms">
<summary><code>reconnect_delay_ms</code>
<span class="setting-meta"><strong>Default:</strong> <code>2000</code></span>
</summary>

Delay between reconnect attempts and startup connect retries (`> 0`).

</details>
</div>

### Publish resilience

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`publish_retry_attempts`](#notification-backend-jetstream-publish-retry-attempts) | `5` |
| [`publish_retry_base_delay_ms`](#notification-backend-jetstream-publish-retry-base-delay-ms) | `150` |

</div>
<details class="setting-panel" id="notification-backend-jetstream-publish-retry-attempts">
<summary><code>publish_retry_attempts</code>
<span class="setting-meta"><strong>Default:</strong> <code>5</code></span>
</summary>

Retries for transient `channel closed` publish failures (`> 0`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-publish-retry-base-delay-ms">
<summary><code>publish_retry_base_delay_ms</code>
<span class="setting-meta"><strong>Default:</strong> <code>150</code></span>
</summary>

Base backoff in ms for publish retries; grows exponentially per attempt (`> 0`).

</details>
</div>

### Stream defaults

These apply to every stream created by Aviso unless overridden by a per-schema
`storage_policy`.

<div class="settings-reference">
<div class="setting-index">

| Setting | Default |
|---|---|
| [`max_messages`](#notification-backend-jetstream-max-messages) | `None` |
| [`max_bytes`](#notification-backend-jetstream-max-bytes) | `None` |
| [`retention_time`](#notification-backend-jetstream-retention-time) | `None` |
| [`storage_type`](#notification-backend-jetstream-storage-type) | `file` |
| [`replicas`](#notification-backend-jetstream-replicas) | `None` |
| [`retention_policy`](#notification-backend-jetstream-retention-policy) | `limits` |
| [`discard_policy`](#notification-backend-jetstream-discard-policy) | `old` |

</div>
<details class="setting-panel" id="notification-backend-jetstream-max-messages">
<summary><code>max_messages</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code></span>
</summary>

Stream message cap (maps to `max_messages`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-max-bytes">
<summary><code>max_bytes</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code></span>
</summary>

Stream size cap in bytes (maps to `max_bytes`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-retention-time">
<summary><code>retention_time</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code></span>
</summary>

Default max age: duration literal (`s`, `m`, `h`, `d`, `w`; e.g. `30d`).

</details>
<details class="setting-panel" id="notification-backend-jetstream-storage-type">
<summary><code>storage_type</code>
<span class="setting-meta"><strong>Default:</strong> <code>file</code></span>
</summary>

`file` or `memory`, parsed as typed enum at config load.

Omitting this setting requests `file`. Storage cannot change on an existing
stream. If its storage differs, operations that ensure the stream fail with the
stream name and current and requested types, before any mutable settings change.
Aviso does not delete or recreate the stream; stored messages are left intact.
Use the stream's current type or arrange a separate migration.

</details>
<details class="setting-panel" id="notification-backend-jetstream-replicas">
<summary><code>replicas</code>
<span class="setting-meta"><strong>Default:</strong> <code>None</code></span>
</summary>

Stream replica count.

</details>
<details class="setting-panel" id="notification-backend-jetstream-retention-policy">
<summary><code>retention_policy</code>
<span class="setting-meta"><strong>Default:</strong> <code>limits</code></span>
</summary>

`limits` or `interest`. `workqueue` is rejected at startup.

</details>
<details class="setting-panel" id="notification-backend-jetstream-discard-policy">
<summary><code>discard_policy</code>
<span class="setting-meta"><strong>Default:</strong> <code>old</code></span>
</summary>

`old` or `new`, parsed as typed enum.

</details>
</div>

> **Fail-fast validation:** `storage_type`, `retention_policy`, and
> `discard_policy` are parsed as typed enums during configuration loading.
> Invalid values fail startup immediately, before any streams are created.

`workqueue` retention is not supported because it cannot support Aviso's
independent watch/replay consumers. Startup rejects it for backend defaults,
including streams with schema storage policies. Schema storage policies inherit
the backend retention policy; they cannot override it. Use `limits` for history
bounded by configured limits. `interest` remains accepted with its existing NATS
interest-based retention semantics; it does not guarantee retained history when
there are no interested consumers. This validation does not migrate or delete
existing streams.

### Full example

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
    timeout_seconds: 30
    retry_attempts: 3
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

On first access (e.g. first publish for a given event type), Aviso creates a
JetStream stream with the following settings applied:

- `storage_type`, `retention_policy`, `discard_policy`
- `max_messages`, `max_bytes`, `retention_time` → `max_age`
- `replicas`

The stream subject binding is set to `<base>.>` (e.g. `mars.>`) to capture all
topics under that base.

### Reconciliation of existing streams

When a stream already exists and is accessed by Aviso, it is **reconciled**: the
current stream config is compared against the desired config and mutable fields
are updated if drift is detected:

- limits (retention, size, message count)
- compression
- duplicate window
- replicas
- subject binding

Aviso leaves settings alone when they already have the intended effect, even
if NATS reports a default differently. Equivalent defaults do not trigger an
update.

If JetStream rejects an update (e.g. the field is not editable in the current
server/stream state), the operation fails. Aviso does not report success while
using stale retention settings. If another replica creates the stream during
creation, Aviso reloads and reconciles that stream before proceeding.

### Precedence

Backend-level defaults are applied first, then per-schema `storage_policy`
overrides for that stream:

Values under `notification_schema.<event_type>.storage_policy` override the
matching `notification_backend.jetstream` defaults.

Policy lookup matches the topic base without regard to ASCII case, just as
startup validation does. NATS subjects themselves remain case-sensitive.
Retention must be positive and fit signed 64-bit nanoseconds. The largest
whole-second literal is `9223372036s`; larger values fail validation.

Shortening retention deletes messages older than the new window, including
messages stored before the change. They are no longer available for replay.
This does not require deleting or recreating the stream. Keep all Aviso replicas
on the same configuration so they do not repeatedly change each other's policy.
When shortening retention below the existing duplicate-detection window, Aviso
also shortens that window to satisfy NATS's limit. A smaller existing window is
preserved.

### Applying config changes to existing streams

Aviso uses the configuration loaded at startup. After editing `config.yaml`,
restart Aviso or roll out the updated configuration to all replicas. Existing
streams are then reconciled when accessed, for example by a publish or a new
watch/replay request. Aviso does not sweep all streams at startup or in the
background.

Compression applies to future file-storage writes at the block level. Changing
the setting does not automatically recompress existing history. Aviso provides
no automatic history migration.

Deleting a stream loses its stored messages. Recreating it starts an empty
stream; it does not rewrite or restore history. The `wipe_stream` admin endpoint
also removes messages, but preserves the stream configuration. Neither is a
compression migration.

---

## Verifying Effective Stream Policy

Use the `nats` CLI to inspect the stream config after a publish or reconcile:

```bash
# Replace POLYGON with your stream name (MARS, DISS, etc.)
nats --server nats://localhost:4222 stream info POLYGON
```

Fields to check:

| CLI field                  | Config field                                       |
| -------------------------- | -------------------------------------------------- |
| `Max Age`                  | `retention_time`                                   |
| `Max Messages`             | `max_messages`                                     |
| `Max Bytes`                | `max_bytes` / per-schema `max_size`                |
| `Max Messages Per Subject` | `allow_duplicates`: `1` = disabled, `-1` = enabled |
| `Compression`              | `None` or `S2`                                     |

---

## Replay Behavior

- **Sequence replay** (`from_id`): starts from that sequence number, inclusive.
- **Time replay** (`from_date`): uses JetStream start-time delivery policy.
- The API enforces mutual exclusivity: `from_id` and `from_date` cannot both be
  present.

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

Run the opt-in reconnect outage test from the repository root:

```bash
bash scripts/test_jetstream_reconnect.sh
```

It requires Docker and uses its own NATS 2.14.6 container with a random
loopback port. It stops and restarts only that container, checks that bounded
reconnect gives up while unset and `0` recover, then removes the container.
It does not use `NATS_URL` or shared NATS storage.

- Startup connectivity is controlled by `timeout_seconds` + `retry_attempts`.
- Runtime reconnect is controlled by `max_reconnect_attempts` and
  `reconnect_delay_ms`.
- Publish retry is a narrow resilience path for transient `channel closed`
  failures; non-transient failures fail fast.
- `retry_attempts` applies only to startup; post-startup reconnect uses the
  reconnect settings.
- Reconnect retries are unlimited unless `max_reconnect_attempts` is set to a
  positive value. A bounded value means the client gives up permanently once
  exhausted and the backend stays disconnected until a process restart, while
  the HTTP surface keeps serving.
- `GET /ready` reflects the connection state: 200 while the NATS connection is
  live, 503 while it is down or reconnecting. Point Kubernetes readiness probes
  at `/ready` so traffic routes away during a backend outage and resumes on
  reconnect; keep liveness on `/health` (process-only) so pods are not killed
  during an outage the client recovers from by itself.
- `max_reconnect_attempts` also bounds subscription-creation retries, where
  unset means 5 attempts and `0` means one attempt (a subscribe call has a
  caller waiting on it, so it never retries forever).
