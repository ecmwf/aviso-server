# ECPDS Plugin Runbook

Use this page to investigate ECPDS authorization problems on watch or replay
requests. For setup, see
[ECPDS Destination Authorization](./authentication.md#ecpds-destination-authorization).

## At a glance

- The plugin is **read-only** (`watch`, `replay`). The `notify` endpoint is
  never gated by ECPDS.
- The plugin **fails closed**: it will never accidentally allow a request. The
  status code distinguishes where the problem is. **`503 Service Unavailable`**
  means the ECPDS check could not reach a verdict (an upstream / partial-outage
  problem); investigate ECPDS and the network. **`500 Internal Server Error`**
  means the plugin itself hit a server-side bug or a misconfiguration on Aviso's
  side (missing `AuthSettings`, no checker registered, an unexpected plugin
  error); investigate Aviso. The full mapping is in the response codes table
  below.
- The plugin **does not retry**. A `503` is the signal to investigate ECPDS; a
  `500` is the signal to investigate Aviso.
- The cache lives in process memory. Restarting Aviso clears it. Replicas have
  independent caches.
- The default `partial_outage_policy` is `strict`: every configured ECPDS
  server must respond successfully or the call fails with 503. A single ECPDS
  server going away takes the whole plugin down. This is intentional. The
  destination list itself is the **union** of every server's response under both
  policies; the choice is purely about how tolerant we are of per-server
  failures.

### Response codes the plugin emits

<div class="settings-reference">
<div class="setting-index">

| HTTP | Where to look |
| --- | --- |
| [`200`](#ecpds-response-200) | Allowed |
| [`403`](#ecpds-response-403) | Authorization |
| [`503`](#ecpds-response-503) | ECPDS or the network |
| [`500`](#ecpds-response-500) | Aviso |

</div>

<details class="setting-panel" id="ecpds-response-200">
<summary><code>200</code> Allowed
<span class="setting-meta">Destination access approved.</span></summary>

The destination is in the user's ECPDS allow-list.

**Tracing event:** `auth.ecpds.check.allowed`

</details>

<details class="setting-panel" id="ecpds-response-403">
<summary><code>403</code> Authorization denied
<span class="setting-meta">Check the requested destination and match
key.</span></summary>

The destination is not in the user's allow-list
(`reason=DestinationNotInList`), or the request omitted the configured
`match_key` field (`reason=MatchKeyMissing`).

**Tracing event:** `auth.ecpds.check.denied`

</details>

<details class="setting-panel" id="ecpds-response-503">
<summary><code>503</code> ECPDS or network failure
<span class="setting-meta">The destination lookup could not reach a
verdict.</span></summary>

The combined ECPDS responses could not satisfy the active
`partial_outage_policy`. Check ECPDS availability, network connectivity and
service-account credentials. The event's `fetch_outcome` field helps narrow
down the cause.

**Tracing event:** `auth.ecpds.check.unavailable`

</details>

<details class="setting-panel" id="ecpds-response-500">
<summary><code>500</code> Aviso error
<span class="setting-meta">Investigate Aviso, not ECPDS.</span></summary>

A server-side bug or local misconfiguration prevented the check. Possible
causes include missing `AuthSettings` or `EcpdsChecker` in `app_data`, or an
unexpected plugin error.

**Tracing event:** `auth.ecpds.check.error`

</details>
</div>

## Symptom and first checks

Start with one affected request and find its plugin event in the logs. An HTTP
403 or 503 alone does not show that ECPDS caused it. Use the same time window
and Aviso replica when comparing logs with metrics. The `username` values show
who was affected, not what caused the problem.

### Watch or replay returns 503

`event_name=auth.ecpds.check.unavailable` confirms that the plugin could not
get a usable destination list under the configured `partial_outage_policy`.

1. Find `event_name=auth.ecpds.fetch.failed` for that user and time. Read
   `server` and `error` to identify the failing ECPDS server and its error.
2. Check that server from the Aviso host, using the configured service account
   and the affected username as the lookup ID. For connection or timeout
   errors, check the URL, DNS and connectivity. For HTTP 401 or 403, verify the
   service account's credentials and access. For other HTTP errors, inspect
   the status: check the URL for 404, throttling for 429, and upstream logs for
   5xx. For an invalid response, inspect the returned body and its `success`
   value rather than assuming the API changed.
3. Check `partial_outage_policy`: `strict` needs every server to succeed;
   `any_success` needs at least one. Use the per-server failure logs to see
   which servers need attention.

**Metrics:**
`aviso_ecpds_access_decisions_total{outcome="unavailable"}` counts requests
that the plugin rejected with 503. `aviso_ecpds_fetch_total`, grouped by
`outcome`, counts fetch attempts across the configured servers, not individual
server calls. Labels include `unreachable`, `http_401`, `http_403`, `http_4xx`,
`http_5xx` and `invalid_response`. The request log uses `fetch_outcome` with
values such as `Unreachable` or `Unauthorized`; the fetch failure log uses
`error`, not `outcome` or `fetch_outcome`.

Failed lookups are not cached, but concurrent requests for the same user can
share one fetch. The request and fetch counters need not rise together. Under
`any_success`, a failure label on the fetch metric can also accompany a usable
list, so it does not by itself mean a request returned 503.

### Watch or replay returns 403

`event_name=auth.ecpds.check.denied` with `reason=DestinationNotInList` means
the requested destination was absent from the list Aviso used for that user.
That list may have come from cache, not a new ECPDS call. If the reason is
`MatchKeyMissing`, use the next section instead.

1. Check the event's `username` and `event_type` against the intended user and
   schema. Compare the request's destination with the configured `match_key`
   and any schema rules that change its value before the check.
2. Query the configured ECPDS servers using Aviso's service account and that
   username as the lookup ID. Check that the destination record has
   `active: true` and a string value in `target_field`. Debug events
   `auth.ecpds.fetch.skipped_inactive` and `auth.ecpds.fetch.skipped_record`
   identify servers whose records were excluded. With `any_success`, also
   check `auth.ecpds.fetch.failed`: a failed server's destinations are absent.
3. Read `cache_outcome` on the denial. `hit` means Aviso reused the user's
   cached list. If ECPDS access was recently changed, retry after
   `cache_ttl_seconds` expires. Check the same replica, since each has its own
   cache.

**Metric:**
`aviso_ecpds_access_decisions_total{outcome="deny_destination"}` counts denied
requests, including repeated checks against a cached list. Those cache hits
do not increase `aviso_ecpds_fetch_total`. This does not tell you how many
users are affected; use the denial logs for that.

### Logs show MatchKeyMissing

`event_name=auth.ecpds.check.denied` with `reason=MatchKeyMissing` means the
configured `match_key` was absent from the processed request identifiers.
The plugin returns 403 before consulting the cache or ECPDS.

1. Use `event_type` to identify the schema. Compare `ecpds.match_key` with its
   identifier field name and the actual request body.
2. Check the deployed version and the configuration loaded at startup.
   Startup validation requires the match key to exist in the schema with
   `required: true`; normal request validation rejects an omitted required
   field before the plugin runs. This event alone does not explain how the
   key went missing.
3. If those settings match, keep the request ID and a redacted request example
   for an Aviso bug report. Investigate how request processing passed
   identifiers without the key to the checker, rather than changing ECPDS
   permissions.

**Metric:**
`aviso_ecpds_access_decisions_total{outcome="deny_match_key_missing"}` counts
these denials. The denial event has `cache_outcome="none"`; this path does not
increase the cache or fetch counters.

### Requests succeed, but no ECPDS checks appear

An absent `auth.ecpds.check.allowed` event does not prove the plugin is off.
Admin requests bypass the destination check, and log filters can hide events.

1. Confirm that you are looking at new watch or replay requests for the
   intended `event_type`, not an already-open stream or `notify` traffic.
   Check the logs and metrics for the replica handling those requests.
2. Check `aviso_ecpds_access_decisions_total{outcome="admin_bypass"}`. An
   increase explains why there are no allow events for admin requests. The
   matching `auth.ecpds.admin.bypass` event is debug-level; the normal
   `auth.ecpds.check.allowed` event is info-level. Check the logging filters.
3. For a non-admin request, verify that the deployed schema's `auth` block
   contains `plugins: ["ecpds"]` and `required: true`. Startup rejects an
   ECPDS plugin reference if the binary lacks the `ecpds` feature or the
   schema has `auth.required: false`; those are not silent bypass settings.

**Metrics:** compare changes in `aviso_ecpds_access_decisions_total` by
`outcome`, not just `allow`. If all `aviso_ecpds_*` series are missing, verify
that metrics are enabled and the scrape reaches the right replica before
checking whether the binary was built with `--features ecpds`.

### Starting a watch or replay is slow

`event_name=auth.ecpds.cache.miss` means a request could not use a cached list.
It may have fetched from ECPDS or waited for another request's fetch. A miss
alone does not prove ECPDS caused the delay.

1. Check `aviso_http_request_duration_seconds` for `route="/api/v1/watch"` or
   `route="/api/v1/replay"`. This measures time until response headers are
   ready, not how long the stream stays open.
2. For a slow request, inspect `cache_outcome` on its `auth.ecpds.check.*`
   result event. `hit` means no upstream fetch was needed; `miss_fetched`
   means this request fetched; `miss_coalesced` means it shared another
   request's fetch. Check `auth.ecpds.fetch.failed` for timeout or connection
   errors. Cache hit/miss and fetch success events require debug logging.
3. Compare the rates of `aviso_ecpds_cache_misses_total` and
   `aviso_ecpds_cache_hits_total` on that replica. Check recent restarts or
   traffic moving between replicas before tuning the cache. Compare
   `cache_ttl_seconds` with how often users reconnect, and
   `aviso_ecpds_cache_size` with `max_entries`. The size gauge is an approximate
   count of cached usernames, not proof of eviction.

Use `aviso_ecpds_fetch_total` to check whether more misses also mean more
fetches; shared fetches count once. If slow requests are cache hits, continue
with the request's other logs rather than assuming the cache needs resizing.

## Tracing event reference

Every event uses the codebase's standard structured shape (`service_name`,
`service_version`, `event_name`, plus event-specific fields). The list below
covers each event with a one-line meaning. Field-value details follow.

<div class="settings-reference">
<div class="setting-index">

| Event | Level |
| --- | --- |
| [`auth.ecpds.check.started`](#ecpds-tracing-check-started) | debug |
| [`auth.ecpds.check.allowed`](#ecpds-tracing-check-allowed) | info |
| [`auth.ecpds.check.denied`](#ecpds-tracing-check-denied) | warn |
| [`auth.ecpds.check.unavailable`](#ecpds-tracing-check-unavailable) | warn |
| [`auth.ecpds.check.error`](#ecpds-tracing-check-error) | error |
| [`auth.ecpds.admin.bypass`](#ecpds-tracing-admin-bypass) | debug |
| [`auth.ecpds.cache.hit`](#ecpds-tracing-cache-hit) | debug |
| [`auth.ecpds.cache.miss`](#ecpds-tracing-cache-miss) | debug |
| [`auth.ecpds.fetch.succeeded`](#ecpds-tracing-fetch-succeeded) | debug |
| [`auth.ecpds.fetch.failed`](#ecpds-tracing-fetch-failed) | warn |
| [`auth.ecpds.fetch.skipped_inactive`](#ecpds-tracing-fetch-skipped-inactive) | debug |
| [`auth.ecpds.fetch.skipped_record`](#ecpds-tracing-fetch-skipped-record) | debug |

</div>
<details class="setting-panel" id="ecpds-tracing-check-started">
<summary><code>auth.ecpds.check.started</code>
<span class="setting-meta"><strong>Level:</strong> debug</span>
</summary>

The plugin started checking access for a request.

</details>
<details class="setting-panel" id="ecpds-tracing-check-allowed">
<summary><code>auth.ecpds.check.allowed</code>
<span class="setting-meta"><strong>Level:</strong> info</span>
</summary>

The plugin allowed the request.

</details>
<details class="setting-panel" id="ecpds-tracing-check-denied">
<summary><code>auth.ecpds.check.denied</code>
<span class="setting-meta"><strong>Level:</strong> warn</span>
</summary>

The plugin denied the request. See `reason` field.

</details>
<details class="setting-panel" id="ecpds-tracing-check-unavailable">
<summary><code>auth.ecpds.check.unavailable</code>
<span class="setting-meta"><strong>Level:</strong> warn</span>
</summary>

The plugin failed to reach a verdict. See `fetch_outcome` field.

</details>
<details class="setting-panel" id="ecpds-tracing-check-error">
<summary><code>auth.ecpds.check.error</code>
<span class="setting-meta"><strong>Level:</strong> error</span>
</summary>

An unexpected error in the plugin. See `error_kind` or `error` field.

</details>
<details class="setting-panel" id="ecpds-tracing-admin-bypass">
<summary><code>auth.ecpds.admin.bypass</code>
<span class="setting-meta"><strong>Level:</strong> debug</span>
</summary>

An admin user skipped the ECPDS check. Demoted from info because admin bypass is
configured behaviour, not an event SREs alert on; the
`aviso_ecpds_access_decisions_total{outcome="admin_bypass"}` Prometheus counter
still records every occurrence unconditionally.

</details>
<details class="setting-panel" id="ecpds-tracing-cache-hit">
<summary><code>auth.ecpds.cache.hit</code>
<span class="setting-meta"><strong>Level:</strong> debug</span>
</summary>

The destination list came from cache.

</details>
<details class="setting-panel" id="ecpds-tracing-cache-miss">
<summary><code>auth.ecpds.cache.miss</code>
<span class="setting-meta"><strong>Level:</strong> debug</span>
</summary>

The destination list was not in cache; a fetch was triggered.

</details>
<details class="setting-panel" id="ecpds-tracing-fetch-succeeded">
<summary><code>auth.ecpds.fetch.succeeded</code>
<span class="setting-meta"><strong>Level:</strong> debug</span>
</summary>

A fetch to one ECPDS server succeeded.

</details>
<details class="setting-panel" id="ecpds-tracing-fetch-failed">
<summary><code>auth.ecpds.fetch.failed</code>
<span class="setting-meta"><strong>Level:</strong> warn</span>
</summary>

A fetch to one ECPDS server failed. See `error` field.

</details>
<details class="setting-panel" id="ecpds-tracing-fetch-skipped-inactive">
<summary><code>auth.ecpds.fetch.skipped_inactive</code>
<span class="setting-meta"><strong>Level:</strong> debug</span>
</summary>

One or more ECPDS records returned by a single server had `active != true`
(false, missing, or not a boolean) and got dropped from the user's allow-list.
Carries `server_index`, `server`, `username`, `skipped`, `total`. Demoted from
info because every ECPDS fetch routinely returns inactive records and the skip
behaviour is the documented contract; flip to debug only when investigating a
denied user whose expected destination appears in this skip count.

</details>
<details class="setting-panel" id="ecpds-tracing-fetch-skipped-record">
<summary><code>auth.ecpds.fetch.skipped_record</code>
<span class="setting-meta"><strong>Level:</strong> debug</span>
</summary>

One or more ECPDS records returned by a single server were active but missing
the configured `target_field` and got dropped. Carries `server_index`, `server`,
`username`, `target_field`, `skipped`, `total` so on-call can pinpoint which
ECPDS server is producing the malformed records. Demoted from info on the same
grounds as `skipped_inactive`.

</details>
</div>

### Common fields

Most events carry `event_type` (the schema name) and `username` (the JWT
subject). Per-server events (`auth.ecpds.fetch.succeeded`, `.failed`, and
`.skipped_record`) also carry `server_index` (zero-based) and `server` (the
parsed URL).

### Field value reference

Some events carry a typed enum field. The values you will see in logs are listed
below. They are spelled exactly as shown.

- `reason` (on `auth.ecpds.check.denied`):
  - `DestinationNotInList`: the user is not entitled to the requested
    destination.
  - `MatchKeyMissing`: the request body did not include the configured
    match-key field.
- `fetch_outcome` (on `auth.ecpds.check.unavailable`):
  - `Unauthorized`, `Forbidden`: an ECPDS server returned 401 or 403.
  - `ClientError`: an ECPDS server returned a 4xx other than 401 or 403
    (commonly 404 for a misconfigured base URL or 429 for throttling).
  - `ServerError`: an ECPDS server returned 5xx.
  - `InvalidResponse`: an ECPDS server returned a body the parser could not
    read.
  - `Unreachable`: network or timeout failure.
- `cache_outcome` (on every `auth.ecpds.check.*` event: `.allowed`, `.denied`,
  `.unavailable`, `.error`):
  - `hit`: served from cache.
  - `miss_coalesced`: the cache was empty for this key but a concurrent
    caller's fetch was in flight; this request waited on it.
  - `miss_fetched`: this request ran the upstream fetch itself. The merged
    per-server result of that fetch is recorded as the `outcome` label on the
    `aviso_ecpds_fetch_total` metric, and on the `auth.ecpds.check.unavailable`
    event also as `fetch_outcome` (see above). It is intentionally NOT inlined
    into `cache_outcome` so log filters keyed on `cache_outcome:miss_fetched`
    stay stable as new `FetchOutcome` variants are added.
  - `none`: cache lookup was deliberately skipped because the request fell at
    the `MatchKeyMissing` deny path before any cache call ran. Only appears on
    `auth.ecpds.check.denied` events alongside `reason=MatchKeyMissing`.

## How to confirm "config error vs. upstream outage"

1. **Is the ECPDS plugin even compiled in?** Check `/metrics` for
   `aviso_ecpds_*` series. The unlabelled counters and gauge plus the
   pre-initialised label values on `aviso_ecpds_access_decisions_total` and
   `aviso_ecpds_fetch_total` register at process startup whenever the binary is
   built with `--features ecpds`, regardless of whether an `ecpds:` config
   block exists. If the series are absent, the binary does not have the feature,
   OR the metrics endpoint itself is disabled (`metrics.enabled: false` in your
   config). If the series exist but
   `aviso_ecpds_access_decisions_total{outcome="allow"}` plus `outcome="deny_*"`
   are all flat at zero under load, the plugin is compiled in but no stream
   actually opts in via `plugins: ["ecpds"]`.
2. **Are the configured server URLs reachable from this Aviso host?** Run this
   from the same host as Aviso:

   ```bash
   curl -i -u "<service-username>:<service-password>" \
        "https://<your-ecpds-host>/ecpds/v1/destination/list?id=<some-test-username>"
   ```

   - `200` with a JSON `destinationList`: ECPDS is up and credentials are
     valid. Problem is on the Aviso side.
   - `401` or `403`: service-account credentials are wrong (rotated, revoked,
     typoed).
   - `5xx` or hang: ECPDS itself is broken.
   - DNS error or connection refused: network-level issue.
3. **Is one specific user being denied while others succeed?** Run the curl
   above with that user's id and compare with the destination they tried to
   read.

## When an ECPDS server is unavailable

With the default `partial_outage_policy: strict`, every configured ECPDS
server must respond successfully when Aviso fetches a user's destination
list. If one fails, requests needing a fresh list receive HTTP 503. Requests
that can use a cached list can still be checked against that list.

With `partial_outage_policy: any_success`, Aviso can use the lists from the
servers that respond successfully. A destination known only to a failed
server may be missing, so a user could be denied access they would normally
have. If no server succeeds, the lookup still fails with HTTP 503.

Both policies combine the destination lists returned by ECPDS. The choice
is whether a lookup requires all servers or at least one. See
[Partial outage policy](./authentication.md#partial-outage-policy) for more
details.
