# ECPDS Plugin Runbook

This page is for the on-call engineer dealing with an ECPDS authorization issue at 3 AM. Read the [ECPDS Destination Authorization](./authentication.md#ecpds-destination-authorization) page first if you haven't already.

## At a glance

- The plugin is **read-only** (`watch`, `replay`). The `notify` endpoint is never gated by ECPDS.
- The plugin **fails closed**. Any internal problem returns `503 Service Unavailable`. The plugin will never accidentally allow a request.
- The plugin **does not retry**. A `503` is the signal to investigate ECPDS, not Aviso.
- The cache lives in process memory. Restarting Aviso clears it. Replicas have independent caches.
- The default `partial_outage_policy` is `strict`. A single ECPDS server going away will take the whole plugin down. This is intentional.

## Symptom and first checks

> **Field-name reading guide.** When this section says `reason=DestinationNotInList`, the actual log line will look like `... reason=DestinationNotInList "ECPDS access denied"`. Use the exact strings shown when grepping. Metric `outcome=...` labels use snake_case (`deny_destination`, `http_401`, etc.).

### 503 storm on watch/replay

- **First metric:** `aviso_ecpds_fetch_total` rate, broken down by `outcome`.
- **First log:** `event_name=auth.ecpds.fetch.failed` and `event_name=auth.ecpds.check.unavailable`.
- **Likely causes** (read off the dominant `outcome` label):
  - `unreachable`: ECPDS server down, network partition, DNS, or wrong `servers` URLs in config.
  - `http_401` or `http_403`: service-account credentials wrong or revoked.
  - `http_5xx`: ECPDS itself is broken.
  - `invalid_response`: ECPDS response shape no longer matches what the parser expects (the contract has changed).
  - `divergence`: strict policy and servers disagree on the user's destination list.

### 403 storm on watch/replay

- **First metric:** `aviso_ecpds_access_decisions_total{outcome="deny_destination"}` rate.
- **First log:** `event_name=auth.ecpds.check.denied` with `reason=DestinationNotInList`.
- **Likely cause:** ECPDS revoked destinations for one or more users. Or a client suddenly started passing the wrong `destination`. Cross-check by hitting the ECPDS web UI directly with the same user.

### 403 with `reason=MatchKeyMissing`

- **First metric:** `aviso_ecpds_access_decisions_total{outcome="deny_match_key_missing"}` rate.
- **First log:** `event_name=auth.ecpds.check.denied` with `reason=MatchKeyMissing`.
- **Likely cause:** the schema's `match_key` field is required, but a client is omitting it. Startup validation should have prevented this configuration in the first place. Investigate config drift.

### Quiet, no allows

- **First metric:** `aviso_ecpds_access_decisions_total{outcome="allow"}` rate is zero.
- **First log:** there isn't one. The plugin is not running.
- **Likely causes:**
  - The binary was built without `--features ecpds`. Startup would have errored if any schema referenced `["ecpds"]`, so this is unlikely on a real deployment.
  - The schema does not actually have `plugins: ["ecpds"]`.
  - `auth.required` is `false` on the schema, so the plugin is unreachable.

### Cache thrashing or latency spike

- **First metric:** ratio of `aviso_ecpds_cache_misses_total` to `aviso_ecpds_cache_hits_total`, plus `aviso_ecpds_cache_size`.
- **First log:** rate of `event_name=auth.ecpds.cache.miss`.
- **Likely cause:** high miss rate with a high number of distinct usernames means `cache_ttl_seconds` is too short, `max_entries` is too small, or there are genuinely many unique users.

### Tracing event `auth.ecpds.fetch.divergence`

- **First metric:** `aviso_ecpds_fetch_total{outcome="divergence"}`.
- **First log:** `event_name=auth.ecpds.fetch.divergence`.
- **Likely cause:** servers report different destination lists for the same user. This is almost always a replication-lag issue at the ECPDS side. Strict policy turns this into a 503. The `any_success` policy takes the union and continues with a warning.

## Tracing event reference

Every event uses the codebase's standard structured shape (`service_name`, `service_version`, `event_name`, plus event-specific fields). The list below covers each event with a one-line meaning. Field-value details follow.

| Event | Level | Meaning |
|-------|-------|---------|
| `auth.ecpds.check.started` | debug | The plugin started checking access for a request. |
| `auth.ecpds.check.allowed` | info | The plugin allowed the request. |
| `auth.ecpds.check.denied` | warn | The plugin denied the request. See `reason` field. |
| `auth.ecpds.check.unavailable` | warn | The plugin failed to reach a verdict. See `fetch_outcome` field. |
| `auth.ecpds.check.error` | error | An unexpected error in the plugin. See `error_kind` or `error` field. |
| `auth.ecpds.admin.bypass` | debug | An admin user skipped the ECPDS check. |
| `auth.ecpds.cache.hit` | debug | The destination list came from cache. |
| `auth.ecpds.cache.miss` | debug | The destination list was not in cache; a fetch was triggered. |
| `auth.ecpds.fetch.succeeded` | debug | A fetch to one ECPDS server succeeded. |
| `auth.ecpds.fetch.failed` | warn | A fetch to one ECPDS server failed. See `error` field. |
| `auth.ecpds.fetch.divergence` | warn | Two or more servers returned different destination lists for the same user. |
| `auth.ecpds.fetch.skipped_record` | info | One or more ECPDS records were missing the configured `target_field` and got dropped. |

### Common fields

Most events carry `event_type` (the schema name) and `username` (the JWT subject). Per-server events also carry `server_index` (zero-based) and `server` (the parsed URL).

### Field value reference

Some events carry a typed enum field. The values you will see in logs are listed below. They are spelled exactly as shown.

- `reason` (on `auth.ecpds.check.denied`):
  - `DestinationNotInList`: the user is not entitled to the requested destination.
  - `MatchKeyMissing`: the request body did not include the configured match-key field.
- `fetch_outcome` (on `auth.ecpds.check.unavailable`):
  - `Unauthorized`, `Forbidden`: an ECPDS server returned 401 or 403.
  - `ServerError`: an ECPDS server returned 5xx.
  - `InvalidResponse`: an ECPDS server returned a body the parser could not read.
  - `Unreachable`: network or timeout failure.
  - `Divergence`: strict policy and servers disagreed.
- `cache_outcome` (on `auth.ecpds.check.allowed`): `Hit` or `Miss`.

### Divergence event field shape (by policy)

`auth.ecpds.fetch.divergence` carries different fields depending on the active policy:

- Under **strict** policy, the fields are `server_index` (the zero-based index of the server that disagreed) and `divergence_count` (how many destinations differed from the canonical set).
- Under **any_success** policy, the fields are `pairwise_divergence_count` (the largest disagreement between any two servers) and `reachable_servers` (how many servers responded).

## How to confirm "config error vs. upstream outage"

1. **Is the ECPDS plugin even running?** Check `/metrics` for `aviso_ecpds_*` series. If they are absent, the binary is not built with `--features ecpds`, or the `ecpds` config block is missing.
2. **Are the configured server URLs reachable from this Aviso host?** Run this from the same host as Aviso:
   ```bash
   curl -i -u "<service-username>:<service-password>" \
        "https://<your-ecpds-host>/ecpds/v1/destination/list?id=<some-test-username>"
   ```
   - `200` with a JSON `destinationList`: ECPDS is up and credentials are valid. Problem is on the Aviso side.
   - `401` or `403`: service-account credentials are wrong (rotated, revoked, typoed).
   - `5xx` or hang: ECPDS itself is broken.
   - DNS error or connection refused: network-level issue.
3. **Is one specific user being denied while others succeed?** Run the curl above with that user's id and compare with the destination they tried to read.

## Blast radius of `partial_outage_policy=strict`

With `strict`, **one** ECPDS server going away takes the whole plugin to 503. Any reader on a stream with `plugins: ["ecpds"]` will see 503 until the missing server returns and agrees with the others.

If your operational priority is availability over confidentiality (e.g. during a known ECPDS replication issue), an explicit and documented switch to `partial_outage_policy: any_success` is the lever. Read the security implication in the [Partial-outage policy](./authentication.md#partial-outage-policy) section before flipping.

## What "the cache is process-local" implies

- Restarting Aviso flushes everyone's destination cache. Expect a brief upstream-call spike right after a restart.
- Multiple Aviso replicas keep independent caches. A user routed to a different replica will see a fresh fetch.
- There is no admin endpoint to flush a single user's cache. The next request after `cache_ttl_seconds` will re-fetch automatically. For an immediate flush, restart the replica.

## What this runbook deliberately does not tell you

- ECPDS API specifics. There is no public ECPDS REST documentation as of this writing. What Aviso assumes about the response shape (e.g. `destinationList[].name`, `success: "yes"`) is captured as automated contract tests under `aviso-ecpds/tests/fixtures/` and `aviso-ecpds/tests/contract.rs`. If those tests start failing on a real ECPDS environment, the contract has changed and Aviso needs an update.
- Kerberos, mTLS, or SSO to ECPDS. Aviso uses HTTP Basic Auth only. Switching to a different auth mechanism would need code changes.
