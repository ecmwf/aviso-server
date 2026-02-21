# JetStream Settings

This page maps `notification_backend.jetstream` settings to runtime behavior.

## Field mapping

| Setting | Default | Applied? | Notes |
|---|---:|---|---|
| `nats_url` | `nats://localhost:4222` | Yes | Used for NATS connection target. |
| `token` | `None` | Yes | Used for token auth (`NATS_TOKEN` env fallback). |
| `timeout_seconds` | `30` | Yes | Applied as NATS `connection_timeout` during connect attempts. |
| `retry_attempts` | `3` | Yes | Controls startup connection retry attempts (`min=1`). |
| `max_messages` | `None` | Yes | Mapped to stream `max_messages`. |
| `max_bytes` | `None` | Yes | Mapped to stream `max_bytes`. |
| `retention_time` | `None` | Yes | Mapped to stream `max_age` using duration literals (`s`, `m`, `h`, `d`, `w`). |
| `storage_type` | `file` | Yes | Parsed at stream creation (`file`/`memory`). |
| `replicas` | `None` | Yes | Mapped to stream `num_replicas`. |
| `retention_policy` | `limits` | Yes | Parsed at stream creation. |
| `discard_policy` | `old` | Yes | Parsed at stream creation. |
| `enable_auto_reconnect` | `true` | Yes | Enables/disables NATS client reconnect behavior. |
| `max_reconnect_attempts` | `5` | Yes | Mapped to NATS `max_reconnects` (`0` means unlimited). |
| `reconnect_delay_ms` | `2000` | Yes | Used for NATS reconnect delay and initial connect retry delay. |
| `publish_retry_attempts` | `5` | Yes | Retry count for transient publish `channel closed` failures (`> 0`). |
| `publish_retry_base_delay_ms` | `150` | Yes | Base backoff in ms for publish retries (`> 0`, exponential by attempt). |

## Important caveats

- Existing streams are reconciled when accessed by Aviso (for example publish path), including
  managed subject binding and mutable policy fields (limits/retention/compression/duplicates/replicas).
- Precedence is backend defaults first, then per-schema `storage_policy` override for the same stream base.
- Invalid values for `storage_type`, `retention_policy`, and `discard_policy` fail during configuration deserialization (startup fail-fast).
- Invalid numeric timing values still fail during backend validation at startup.
- `retry_attempts` applies to startup connect attempts; reconnect behavior after startup uses
  `enable_auto_reconnect`/`max_reconnect_attempts`.
- Publish retries are a narrow resilience path for transient `channel closed` transport failures;
  non-transient publish failures fail fast.

## Verify with nats CLI

```bash
# Replace stream name as needed (for example DISS, MARS, POLYGON)
nats --server nats://localhost:4222 stream info POLYGON
```

Check `Max Age`, `Max Messages`, `Max Bytes`, `Max Messages Per Subject`, and `Compression`.

## Recommended usage

- Use `retry_attempts` to control startup resiliency when NATS is temporarily unavailable.
- Set `max_reconnect_attempts = 0` only if you want unlimited reconnect retries.
- Tune `publish_retry_attempts` and `publish_retry_base_delay_ms` when your network/NATS path is bursty.
- Use stable policy values:
  - `storage_type`: `file` or `memory`
  - `retention_policy`: `limits`, `interest`, `workqueue`
  - `discard_policy`: `old`, `new`

## Example

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
