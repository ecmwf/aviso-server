# Backend Development Guide

This guide explains how to add a new notification backend in Aviso.

## Required Contract

A backend must implement `NotificationBackend` in
`src/notification_backend/mod.rs`.

Core requirements:

- Implement publish/replay/subscribe/admin methods.
- Implement `capabilities()` and return a stable `BackendCapabilities` map.
- Keep startup/shutdown behavior explicit and logged.

`subscribe_to_topic` returns `Subscription { stream, history_end }`. Capture the
receiver and inclusive history bound at the same logical creation point as
publishing. The returned live stream must start strictly above that bound.
Never combine a subscription with a later, independently sampled stream tail.
`history_end(topic)` captures the replay-only bound without a live subscription.

`get_messages_batch` must enforce `BatchParams.end_sequence` before filtering,
rendering or pagination. Preserve that bound when advancing the start cursor.
Completion must not depend on finding a message exactly at the bound: it may be
deleted, overwritten or excluded. The sequence range is finite, but storage
contents can still change during replay.

## Storage Policy Compatibility

Per-schema storage policy is validated at startup before backend initialization.

Validation entry point:

- `configuration::validate_schema_storage_policy_support(...)`

Capability source:

- `notification_backend::capabilities_for_backend_kind(...)`

If a schema requests unsupported fields, startup fails fast with a clear error.
Do not silently ignore unsupported storage policy fields.

## Capability Checklist

When adding backend `<new_backend>`:

1. Add backend kind support in `capabilities_for_backend_kind`.
2. Add `NotificationBackend::capabilities()` implementation.
3. Ensure capability values match real backend behavior.
4. Add tests for:
   - capability map values
   - accepted storage-policy fields
   - rejected storage-policy fields
5. Add backend docs page and update summary links.

## Minimal Capability Example

```text
BackendCapabilities {
    retention_time: true,
    max_messages: true,
    max_size: false,
    allow_duplicates: false,
    compression: false,
}
```

Meaning:

- `retention_time` and `max_messages` can be used in schema storage policy.
- `max_size`, `allow_duplicates`, and `compression` must be rejected at startup.

## Testing Expectations

- Unit tests should verify capability flags are stable.
- Validation tests should verify fail-fast messages for unsupported fields.
- Integration tests should use test-local config/schema fixtures, not
  developer-local YAML files.

Run `bash scripts/test_replay_boundary.sh` for the full test suite with live
JetStream coverage on an isolated NATS 2.14.6 container. Add `--features ecpds`
for that build. The script allocates a loopback port and removes its own server
and disposable storage on exit; it does not use an existing NATS instance.
