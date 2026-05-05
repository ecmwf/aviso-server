# ECPDS Plugin Runbook

This page is for the on-call engineer dealing with an ECPDS authorization issue at 3 AM. It assumes the [ECPDS Destination Authorization](./authentication.md#ecpds-destination-authorization) page has already been read at least once.

## At a glance

- The plugin is **read-only** (`watch`, `replay`). `notify` is never gated by ECPDS.
- The plugin **fails closed**: any internal problem returns `503 Service Unavailable`, not an accidental allow.
- The plugin **does not retry**. A `503` is the signal to investigate ECPDS, not Aviso.
- The cache is **process-local**. Restarting Aviso clears it; replicas have independent caches.
- The default `partial_outage_policy` is `strict`: a single ECPDS server going away takes the whole plugin down. This is intentional.

## Symptom → first checks

> **Field-name reading guide.** Where this section says `reason=DestinationNotInList`, the actual log line will look like `… reason=DestinationNotInList "ECPDS access denied"` because the field is rendered with Rust's `Debug` formatter from the typed `DenyReason` enum. Use the exact strings shown when grepping. Metric `outcome=…` labels are the snake_case strings (`deny_destination`, `http_401`, etc.).

### 503 storm on watch/replay

- **First metric:** `aviso_ecpds_fetch_total` rate, broken down by `outcome`.
- **First log:** `event_name=auth.ecpds.fetch.failed` and `event_name=auth.ecpds.check.unavailable`.
- **Likely causes** (read off the dominant `outcome` label):
  - `unreachable` — ECPDS server down, network partition, DNS, or wrong `servers` URLs in config.
  - `http_401` / `http_403` — service-account credentials wrong or revoked.
  - `http_5xx` — ECPDS itself is broken.
  - `invalid_response` — ECPDS response shape no longer matches what the parser expects (contract drift).
  - `divergence` — strict policy and servers disagree on the user's destination list.

### 403 storm on watch/replay

- **First metric:** `aviso_ecpds_access_decisions_total{outcome="deny_destination"}` rate.
- **First log:** `event_name=auth.ecpds.check.denied` with `reason=DestinationNotInList`.
- **Likely causes:** ECPDS revoked destinations for one or more users; or a client suddenly started passing the wrong `destination`. Cross-check by hitting the ECPDS web UI directly with the same user.

### 403 with `reason=MatchKeyMissing`

- **First metric:** `aviso_ecpds_access_decisions_total{outcome="deny_match_key_missing"}` rate.
- **First log:** `event_name=auth.ecpds.check.denied` with `reason=MatchKeyMissing`.
- **Likely cause:** the schema's `match_key` field is required, but a client is omitting it. Startup validation should have prevented this configuration in the first place — investigate config drift.

### Quiet / no allows

- **First metric:** `aviso_ecpds_access_decisions_total{outcome="allow"}` rate is zero.
- **First log:** there isn't one — the plugin simply isn't running.
- **Likely causes:**
  - The binary was built without `--features ecpds`. Startup would have errored if any schema referenced `["ecpds"]`, so this is unlikely on a real deployment.
  - The schema does not actually have `plugins: ["ecpds"]`.
  - `auth.required` is `false` on the schema, so the plugin is unreachable.

### Cache thrashing / latency spike

- **First metric:** ratio of `aviso_ecpds_cache_misses_total` to `aviso_ecpds_cache_hits_total`, plus `aviso_ecpds_cache_size`.
- **First log:** rate of `event_name=auth.ecpds.cache.miss`.
- **Likely cause:** high miss rate with high cardinality of distinct usernames means `cache_ttl_seconds` is too short, `max_entries` is too small, or there are genuinely many unique users.

### Tracing event `auth.ecpds.fetch.divergence`

- **First metric:** `aviso_ecpds_fetch_total{outcome="divergence"}`.
- **First log:** `event_name=auth.ecpds.fetch.divergence`.
- **Likely cause:** servers report different destination lists for the same user. Almost always a replication-lag issue at the ECPDS side. Strict policy turns this into a 503; `AnySuccess` policy takes the union and continues with a warning.

## Tracing event reference

All events use the codebase's standard structured shape (`service_name`, `service_version`, `event_name`, plus event-specific fields).

| `event_name` | Level | Fired by | Notable fields |
|---|---|---|---|
| `auth.ecpds.check.started` | debug | `enforce_ecpds_auth` | `event_type`, `username` |
| `auth.ecpds.check.allowed` | info | `enforce_ecpds_auth` | `event_type`, `username`, `cache_outcome` |
| `auth.ecpds.check.denied` | warn | `enforce_ecpds_auth` | `event_type`, `username`, `reason` ∈ {`DestinationNotInList`, `MatchKeyMissing`} (Rust Debug form) |
| `auth.ecpds.check.unavailable` | warn | `enforce_ecpds_auth` | `event_type`, `username`, `fetch_outcome` ∈ {`Success`, `Unauthorized`, `Forbidden`, `ServerError`, `InvalidResponse`, `Unreachable`, `Divergence`} (Rust Debug form) |
| `auth.ecpds.check.error` | error | `enforce_ecpds_auth` | `event_type`, `error_kind` (or `error`) |
| `auth.ecpds.admin.bypass` | debug | `enforce_ecpds_auth` | `event_type`, `username` |
| `auth.ecpds.cache.hit` | debug | `EcpdsChecker` | `username` |
| `auth.ecpds.cache.miss` | debug | `EcpdsChecker` | `username` |
| `auth.ecpds.fetch.succeeded` | debug | `EcpdsClient` | `server_index`, `server`, `username` |
| `auth.ecpds.fetch.failed` | warn | `EcpdsClient` | `server_index`, `server`, `username`, `error` |
| `auth.ecpds.fetch.divergence` (strict) | warn | `EcpdsClient` strict merge | `server_index`, `divergence_count` |
| `auth.ecpds.fetch.divergence` (any_success) | warn | `EcpdsClient` any_success merge | `pairwise_divergence_count`, `reachable_servers` |
| `auth.ecpds.fetch.skipped_record` | info | `EcpdsClient` | `target_field`, `skipped`, `total` |

## How to confirm "config error vs upstream outage"

1. **Is the ECPDS plugin even running?** Check `/metrics` for `aviso_ecpds_*` series. If they are absent, the binary is not built with `--features ecpds`, or the `ecpds` config block is missing.
2. **Are the configured server URLs reachable from this Aviso replica?** From the host running Aviso:
   ```bash
   curl -i -u "<service-username>:<service-password>" \
        "https://<your-ecpds-host>/ecpds/v1/destination/list?id=<some-test-username>"
   ```
   - `200` with a JSON `destinationList` → ECPDS is up and creds are valid; problem is in Aviso.
   - `401`/`403` → service-account creds are wrong (rotated, revoked, typoed).
   - `5xx` or hang → ECPDS itself is broken.
   - DNS/connection refused → network-level issue.
3. **Is one specific user being denied while others succeed?** Check the user's destinations directly with the curl above (passing `id=<that-user>`); compare with the destination they tried to read from Aviso.

## Blast radius of `partial_outage_policy=strict`

With `strict`, **one** ECPDS server going away takes the whole plugin to 503. Any reader on a stream with `plugins: ["ecpds"]` will see 503 until the missing server returns and agrees with the others.

If your operational priority is availability over confidentiality (e.g. during a known ECPDS replication issue), an explicit, documented switch to `partial_outage_policy: any_success` is the lever. Note the security implication in [Partial-outage policy](./authentication.md#partial-outage-policy) before flipping.

## What "the cache is process-local" implies

- Restarting Aviso flushes everyone's destination cache. Expect a brief upstream-call spike right after a restart.
- Multiple Aviso replicas keep independent caches. A user routed to a different replica will see a fresh fetch.
- There is no admin endpoint to flush a single user's cache. The next request after `cache_ttl_seconds` re-fetches automatically; for an immediate flush, restart the replica.

## What this runbook deliberately does not tell you

- ECPDS API specifics. There is no public ECPDS REST documentation as of this writing; what Aviso assumes about the response shape (e.g. `destinationList[].name`, `success: "yes"`) is captured as executable contract tests under `aviso-ecpds/tests/fixtures/` and `aviso-ecpds/tests/contract.rs`. If those tests start failing on a real ECPDS environment, the contract has changed and Aviso needs an update.
- Kerberos / mTLS / SSO to ECPDS. Aviso uses HTTP Basic Auth only; switch to a different auth mechanism would need code changes.
