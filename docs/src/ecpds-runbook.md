# ECPDS Plugin Runbook

This page is for the on-call engineer dealing with an ECPDS authorization issue at 3 AM. Read the [ECPDS Destination Authorization](./authentication.md#ecpds-destination-authorization) page first if you haven't already.

## At a glance

- The plugin is **read-only** (`watch`, `replay`). The `notify` endpoint is never gated by ECPDS.
- The plugin **fails closed**. Any internal problem returns `503 Service Unavailable`. The plugin will never accidentally allow a request.
- The plugin **does not retry**. A `503` is the signal to investigate ECPDS, not Aviso.
- The cache lives in process memory. Restarting Aviso clears it. Replicas have independent caches.
- The default `partial_outage_policy` is `strict`: every configured ECPDS server must respond successfully or the call fails with 503. A single ECPDS server going away takes the whole plugin down. This is intentional. The destination list itself is the **union** of every server's response under both policies; the choice is purely about how tolerant we are of per-server failures.

## Symptom and first checks

> **What "a storm of X" means here.** Throughout this section, "503 storm" or "403 storm" means: the **rate** of that response is far above the normal baseline. The rule of thumb is, if your dashboard shows the rate climbing fast or staying high for more than a minute or two, treat it as a storm. The first metric and first log lines below are how you confirm it.
>
> **Metrics count requests, not users.** Every counter in this section increments per request. A single client retrying in a tight loop can inflate any of them, so the metric alone cannot tell you whether you are looking at one bad client or hundreds of unhappy users. To answer that, grep the relevant warn or error event in your logs and count distinct values of the `username` field. Every `auth.ecpds.check.*` and `auth.ecpds.fetch.*` warn/error event carries `username` as a structured field for exactly this purpose.
>
> **One useful tell.** Successful and `deny_destination` results are cached for the user; errors are not. So for `deny_destination`, retries by the same user keep adding to `aviso_ecpds_access_decisions_total{outcome="deny_destination"}` but stop adding to `aviso_ecpds_fetch_total` until the TTL expires. If `access_decisions_total` is climbing fast and `fetch_total` is flat, you are almost certainly seeing one or a few users retrying against a cached deny rather than a real population spike. For `unavailable` (503), errors are not cached, so retries do reach ECPDS and both counters move together.
>
> **Field-name reading guide.** When this section says `reason=DestinationNotInList`, the actual log line will look like `... reason=DestinationNotInList "ECPDS access denied"`. Use the exact strings shown when grepping. Metric `outcome=...` labels use snake_case (`deny_destination`, `http_401`, etc.).

### 503 storm on watch/replay

- **What you see:** sustained HTTP 503 responses on `/api/v1/watch` and `/api/v1/replay`. From a user's perspective: "I cannot start a watch; Aviso says ECPDS is unaccessible."
- **Why it happens:** the ECPDS plugin tried to fetch destination lists from your ECPDS servers and could not reach a verdict, so it failed safely with 503 rather than guess.
- **First metric:** `aviso_ecpds_fetch_total` rate, broken down by `outcome`.
- **First log:** `event_name=auth.ecpds.fetch.failed` and `event_name=auth.ecpds.check.unavailable`.
- **Confirm scope:** count distinct `username` values in `event_name=auth.ecpds.check.unavailable` log lines. Many distinct usernames means an ECPDS-side or network problem. One or two usernames means a single misbehaving client is moving the metric.
- **Likely causes** (read off the dominant `outcome` label):
  - `unreachable`: ECPDS server down, network partition, DNS, or wrong `servers` URLs in config.
  - `http_401` or `http_403`: service-account credentials wrong or revoked.
  - `http_5xx`: ECPDS itself is broken.
  - `invalid_response`: ECPDS response shape no longer matches what the parser expects (the contract has changed).


### 403 storm on watch/replay

- **What you see:** sustained HTTP 403 responses on `/api/v1/watch` and `/api/v1/replay`. From a user's perspective: "I used to be able to read this destination, now Aviso says I'm not allowed."
- **Why it happens:** authentication is fine (otherwise it would be a 401), and Aviso did reach ECPDS (otherwise it would be a 503), but ECPDS replied that the user does not have the requested destination on their list.
- **First metric:** `aviso_ecpds_access_decisions_total{outcome="deny_destination"}` rate.
- **First log:** `event_name=auth.ecpds.check.denied` with `reason=DestinationNotInList`.
- **Confirm scope:** count distinct `username` values in those log lines. If you see many distinct users, the cause is upstream of Aviso (ECPDS revoked destinations for several users, or a client batch suddenly started passing the wrong `destination`). If you see one or two, focus on those clients. Cross-check by hitting the ECPDS web UI directly with the same user.

### 403 with `reason=MatchKeyMissing`

- **What you see:** any 403 on `/api/v1/watch` or `/api/v1/replay` whose log carries `reason=MatchKeyMissing`. Even one of these is suspicious; a stream of them means a misconfigured deployment is in production.
- **Why it happens:** the request body did not include the configured match-key field (e.g. no `destination` value at all). The plugin can't check what isn't there, so it denies. Startup validation enforces that this field is `required: true` in the schema, so the only way to see this in practice is if the running config has drifted from what was validated.
- **First metric:** `aviso_ecpds_access_decisions_total{outcome="deny_match_key_missing"}` rate.
- **First log:** `event_name=auth.ecpds.check.denied` with `reason=MatchKeyMissing`.
- **Likely cause:** the schema's `match_key` field is required, but a client is omitting it. Startup validation should have prevented this configuration in the first place. Investigate config drift.

### Quiet, no allows

- **What you see:** ECPDS-protected reads are happening (you see `watch`/`replay` traffic in your access logs and they return 200), but the `aviso_ecpds_access_decisions_total{outcome="allow"}` counter stays flat at zero, and you never see `event_name=auth.ecpds.check.allowed` in logs.
- **Why it happens:** the plugin is not running for those reads. Either the build doesn't include it, or the schema isn't wired up to use it.
- **First metric:** `aviso_ecpds_access_decisions_total{outcome="allow"}` rate is zero.
- **First log:** there isn't one. The plugin is not running.
- **Likely causes:**
  - The binary was built without `--features ecpds`. Startup would have errored if any schema referenced `["ecpds"]`, so this is unlikely on a real deployment.
  - The schema does not actually have `plugins: ["ecpds"]`.
  - `auth.required` is `false` on the schema, so the plugin is unreachable.

### Cache thrashing or latency spike

- **What you see:** average and p99 latency on `/api/v1/watch` and `/api/v1/replay` are climbing, and the cache miss counter is rising significantly faster than the cache hit counter. From a user's perspective: "My watches and replays feel slower than usual."
- **Why it happens:** "cache thrashing" means the cache is barely helping. Most requests are missing the cache and Aviso ends up making a fresh ECPDS call on every request. That call adds latency to every request and load to ECPDS. Possible reasons: cache TTL is too short and entries expire before they're reused; cache is too small and entries get evicted before reuse; or there are genuinely so many distinct usernames that no cache size would fit them all.
- **First metric:** ratio of `aviso_ecpds_cache_misses_total` to `aviso_ecpds_cache_hits_total`, plus `aviso_ecpds_cache_size`.
- **First log:** rate of `event_name=auth.ecpds.cache.miss`.
- **Likely cause:** high miss rate with a high number of distinct usernames means `cache_ttl_seconds` is too short, `max_entries` is too small, or there are genuinely many unique users.

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
- `cache_outcome` (on `auth.ecpds.check.allowed`): `Hit` or `Miss`.

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

With `strict`, **one** ECPDS server going away takes the whole plugin to 503. Any reader on a stream with `plugins: ["ecpds"]` will see 503 until the missing server returns. The destination list itself would still be a union once both servers respond again; the policy only governs how strictly we treat per-server failures.

If you would rather keep serving requests during a partial outage at the cost of possibly missing entitlements that lived only on the unreachable server, switch to `partial_outage_policy: any_success`. Read the trade-off in the [Partial-outage policy](./authentication.md#partial-outage-policy) section before flipping.

## What "the cache is process-local" implies

- Restarting Aviso flushes everyone's destination cache. Expect a brief upstream-call spike right after a restart.
- Multiple Aviso replicas keep independent caches. A user routed to a different replica will see a fresh fetch.
- There is no admin endpoint to flush a single user's cache. The next request after `cache_ttl_seconds` will re-fetch automatically. For an immediate flush, restart the replica.

## What this runbook deliberately does not tell you

- ECPDS API specifics. There is no public ECPDS REST documentation as of this writing. What Aviso assumes about the response shape (e.g. `destinationList[].name`, `success: "yes"`) is captured as automated contract tests under `aviso-ecpds/tests/fixtures/` and `aviso-ecpds/tests/contract.rs`. If those tests start failing on a real ECPDS environment, the contract has changed and Aviso needs an update.
- Kerberos, mTLS, or SSO to ECPDS. Aviso uses HTTP Basic Auth only. Switching to a different auth mechanism would need code changes.
