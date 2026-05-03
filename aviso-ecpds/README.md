# aviso-ecpds

ECPDS (ECMWF Production Data Service) destination authorization plugin for `aviso-server`.

This crate is consumed by `aviso-server` as an **optional path-dependency** behind the `ecpds` Cargo feature. It lives in its own crate so deployments that don't need ECPDS authorization compile a binary with zero ECPDS code, including zero `reqwest` calls to ECPDS-specific endpoints.

## What it does

When a user calls `watch` or `replay` on a stream whose schema declares `auth.plugins: ["ecpds"]`, this crate:

1. Extracts the configured `match_key` value (e.g. `destination`) from the request.
2. Looks up the user's destination list in an in-process single-flight bounded cache (TTL + size cap, eviction via moka's TinyLFU). On miss, queries the configured ECPDS servers in parallel.
3. Merges per-server results under the configured [`PartialOutagePolicy`](src/config.rs).
4. Allows the request iff the requested destination is in the user's authorized list. Otherwise denies with a typed [`DenyReason`](src/client.rs).

`notify` is never gated.

## Public API

- [`config::EcpdsConfig`] / [`config::PartialOutagePolicy`] — serde-deserialized configuration.
- [`checker::EcpdsChecker`] — fallible `new`, async `check_access`, `cache_entry_count` for metric sampling.
- [`client::EcpdsError`] — domain error enum with typed [`client::FetchOutcome`] (`success` / `http_401` / `http_403` / `http_5xx` / `invalid_response` / `unreachable` / `divergence`) and [`client::DenyReason`] (`DestinationNotInList`, `MatchKeyMissing`). Both have stable Prometheus label strings for the `aviso_ecpds_*` metrics in `aviso-server`.
- [`cache::CacheOutcome`] — `Hit` or `Miss`, returned alongside `check_access` results so the route layer can label cache hit-rate metrics.

This crate is **framework-agnostic** by design: it does not depend on `actix-web`, `aviso-server`, or `prometheus`. The route layer in `aviso-server` is responsible for HTTP response shaping and metric recording. This keeps the boundary between "decide" (here) and "expose" (there) clean.

## ECPDS API contract assumptions

ECPDS has no public REST API documentation as of this writing. The contract this crate assumes is:

- `GET <server>/ecpds/v1/destination/list?id=<username>` with HTTP Basic Auth (service account credentials).
- 200 response body parsed as `{"destinationList": [<record>, ...], "success": "<string>"}`.
- Each record is treated as a JSON object; the configured `target_field` (default `"name"`) is extracted as a UTF-8 string. Records that lack the field are silently skipped.
- The `success` field is currently ignored — only `destinationList` content is consulted. (See `tests/contract.rs::success_no_fixture_currently_treated_as_empty_list` for the explicit semantics.)
- 4xx/5xx responses are surfaced as `EcpdsError::Http { status, .. }`; the merge layer maps them to `FetchOutcome::Unauthorized`/`Forbidden`/`ServerError` so SREs can distinguish "creds wrong" from "ECPDS down".

These assumptions are pinned by the captured-fixture tests under [`tests/fixtures/`](tests/fixtures/) plus the integration tests in [`tests/contract.rs`](tests/contract.rs). **If a real ECPDS environment ever produces a response shape that breaks those tests, the contract has changed and this crate needs an update.** That is the single failing test to look for.

## Test fixtures

| Fixture | Asserts |
|---------|---------|
| `populated_user.json` | Three destinations (one inactive); `name` field present on each. |
| `empty_user.json` | Empty `destinationList` with `success: "yes"` denies all destinations. |
| `success_no.json` | `success: "no"` is currently treated as the literal (empty) `destinationList`, NOT as a server-side failure. |
| `record_missing_target_field.json` | Records lacking `target_field` are silently skipped, not surfaced as destinations. |

## Cargo features

- `default = []` (no features).
- The crate has no feature flags of its own. The `ecpds` feature gating happens on the **parent crate** (`aviso-server`), which optionally pulls this crate in.

## Why this crate is path-dep, not workspace-member

`aviso-server` follows the same convention as `aviso-validators`: domain-specific support crates live as path-dependencies rather than workspace members so they can be enabled/disabled cleanly via parent-crate feature flags without affecting the workspace lockfile or the default build.

## Related documentation

- [Authentication > ECPDS Destination Authorization](../docs/src/authentication.md#ecpds-destination-authorization) — operator-facing setup guide.
- [ECPDS Plugin Runbook](../docs/src/ecpds-runbook.md) — on-call triage.
- [Configuration Reference > `ecpds`](../docs/src/configuration-reference.md#ecpds) — every config field.

## License

Apache-2.0
