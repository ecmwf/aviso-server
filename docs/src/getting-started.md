# Getting Started

This guide walks you through running Aviso Server locally and sending your first notification.
It assumes you have already completed [Installation](./installation.md).

If you haven't read [Key Concepts](./concepts.md) yet, do that first; it will make the commands below much easier to follow.

---

## 1. Choose a Backend

Aviso needs a storage backend before it can accept notifications.
For local exploration, **in-memory** requires zero infrastructure.

| Goal | Backend |
|---|---|
| Quick local test, no setup | `in_memory` |
| Persistent history, realistic behavior | `jetstream` |

The examples on this page use `in_memory`.
To use JetStream locally, see [Local JetStream setup](#optional-local-jetstream-setup) below.

---

## 2. Configure the Server

Open `configuration/config.yaml` and make sure it contains at minimum:

```yaml
application:
  host: "127.0.0.1"
  port: 8000
  base_url: "http://localhost:8000"

notification_backend:
  kind: in_memory
  in_memory:
    max_history_per_topic: 100
    max_topics: 10000

notification_schema:
  my_event:
    topic:
      base: "my_event"
      key_order: ["region", "date"]
    identifier:
      region:
        description: "Geographic region label."
        type: EnumHandler
        values: ["north", "south", "east", "west"]
        required: true
      date:
        type: DateHandler
        required: true
    payload:
      required: false
```

> You can also use environment variables to override any config value without editing the file.
> See [Configuration](./configuration.md) for the full precedence rules.

---

## 3. Start the Server

```bash
cargo run
```

Or with the release binary:

```bash
./target/release/aviso_server
```

You should see structured JSON log output. Once you see a line like:

```json
{"level":"INFO","message":"aviso-server listening","address":"127.0.0.1:8000"}
```

the server is ready.

---

## 4. Check the Health Endpoint

```bash
curl -sS http://127.0.0.1:8000/health
```

Expected response: `200 OK`

---

## 5. Open a Watch Stream

Before publishing, open a terminal and start watching for events.
This is a live SSE stream; keep it open while you proceed to step 6.

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/watch" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "my_event",
    "identifier": {
      "region": "north",
      "date":   "20250706"
    }
  }'
```

You will see the SSE connection frame immediately:

```
event: live-notification
data: {"connection_will_close_in_seconds":3600,"request_id":"a3f1d2c8-9b4e-4f7a-bd56-1c8e2a9d4e3f","timestamp":"2026-03-04T10:00:00Z","topic":"my_event.north.20250706","type":"connection_established"}
```

The `request_id` here is unique to this watch request. Save it: you will compare it with the next request's `request_id` to confirm each call gets its own.

The stream stays open and will print new events as they arrive.

---

## 6. Publish a Notification

In a second terminal, send a notification:

```bash
curl -sS -X POST "http://127.0.0.1:8000/api/v1/notification" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "my_event",
    "identifier": {
      "region": "north",
      "date":   "20250706"
    },
    "payload": { "note": "data is ready" }
  }'
```

Expected response:

```json
{
  "status": "success",
  "request_id": "0d4f6758-1ce3-4dda-a0f3-0ccf5fcb50d6",
  "processed_at": "2026-03-04T10:00:00Z"
}
```

`request_id` is the per-request UUID for this notify call. Note that it is **different** from the watch call's UUID above (`a3f1d2c8-...`); each HTTP request gets its own. The same value appears in the `X-Request-ID` HTTP response header and in the corresponding server log lines. Quote it when reporting issues.

Switch back to the watch terminal. The notification should have arrived as a CloudEvent body:

```
event: live-notification
data: {"data":{"identifier":{...},"payload":{"note":"data is ready"}},"datacontenttype":"application/json","dataschema":"http://localhost:8000/schema/my_event","id":"my_event@1","source":"http://localhost:8000","specversion":"1.0","time":"2026-03-04T10:00:00.123456Z","type":"int.ecmwf.aviso.my_event"}
```

The `source` and `dataschema` fields are derived from the server's `application.base_url`. With the default config (no `base_url` set), they would use `http://localhost`; the example above matches the snippet in step 2 which sets it to `http://localhost:8000`. The CloudEvent body also includes the canonicalized identifier and the payload under `data`.

The CloudEvent `id` (`my_event@1`) is the `<event_type>@<sequence>` reference for replay and admin delete. The replay endpoint takes only the numeric sequence (`"1"`), not the full string.

---

## 7. Replay History

Once you have published a few notifications, you can replay them from a specific point:

```bash
curl -N -X POST "http://127.0.0.1:8000/api/v1/replay" \
  -H "Content-Type: application/json" \
  -d '{
    "event_type": "my_event",
    "identifier": {
      "region": "north",
      "date":   "20250706"
    },
    "from_id": "1"
  }'
```

The stream will emit all matching historical notifications, then close with:

```
event: connection-closing
data: {"message":"Stream completed","reason":"end_of_stream","request_id":"f7e8d910-2b3c-4d5a-9c8b-1234567890ab","timestamp":"...","topic":"my_event.north.20250706"}
```

This `request_id` is yet another distinct UUID, since `/replay` is a separate HTTP request from `/watch` and `/notification`.

---

## Optional: Local JetStream Setup

To test with the JetStream backend (persistent storage, more realistic), see
[Installation: Local JetStream](./installation.md#local-jetstream-docker) for the full setup
including environment variables and token authentication.

---

## Run the Smoke Test

A Python smoke script covers the full notify → watch → replay cycle.
Copy the example config and start the server:

```bash
cp configuration/config.yaml.example configuration/config.yaml
cargo run
```

**With auth (default).** Start auth-o-tron before running the smoke tests:

```bash
python3 -m pip install httpx
./scripts/auth-o-tron-docker.sh
python3 scripts/smoke_test.py
```

**Without auth.** Set `auth.enabled: false` in your config (or remove the `auth` section), then:

```bash
AUTH_ENABLED=false python3 scripts/smoke_test.py
```

`AUTH_ENABLED` must match the server's `auth.enabled` setting.
When `false`, auth headers are omitted and auth-specific smoke tests are skipped.

Useful overrides:

```bash
BASE_URL="http://127.0.0.1:8000" python3 scripts/smoke_test.py
BACKEND="jetstream"               python3 scripts/smoke_test.py
TIMEOUT_SECONDS=12                python3 scripts/smoke_test.py
SMOKE_VERBOSE=1                   python3 scripts/smoke_test.py
python3 scripts/smoke_test.py --verbose
```

The smoke script covers:

- health endpoint
- replay/watch baseline flows (`test_polygon`)
- `mars` replay with dot-containing identifier values, integer and enum predicates
- `dissemination` watch + `from_date` with dot-containing identifier values
- read/write auth separation across public, role-restricted, and admin-only streams
- (optional, off by default) ECPDS plugin allow/deny/notify-bypass (see below)

### Optional: end-to-end ECPDS plugin smoke test

If your build has `--features ecpds` enabled and your config has a working `ecpds:` block pointing at your real ECPDS servers, the smoke script can verify the plugin end-to-end against that ECPDS. It's off by default.

**Prerequisites:**

- The server must run with `auth.enabled: true` and `auth.mode: direct`. The smoke script sends HTTP Basic credentials, which Aviso forwards to auth-o-tron only in **direct mode**. Trusted-proxy mode would require an upstream proxy to mint a JWT, which is out of scope for the smoke script.
- Your auth-o-tron config must know two users: an admin user (defaults `admin-user` / `admin-pass`) for the NOTIFY-bypass case, and your ECPDS user (`ECPDS_ALLOWED_USER` / `ECPDS_ALLOWED_PASS`) for the watch cases.
- You need a destination value the ECPDS user **is** entitled to and one they are **not** (the latter can be a deliberately-fake string).
- **Add a minimal ECPDS test schema to your config**, with `match_key` (e.g. `destination`) as the *only* required identifier field. The smoke test sends a minimal request body and does not populate any other required identifier fields. Don't point it at a richer schema like your production `dissemination`. Add this dedicated test schema instead:

  ```yaml
  notification_schema:
    ecpds_test:
      payload:
        required: false
      topic:
        base: "ecpds_test"
        key_order: ["destination"]
      identifier:
        destination:
          type: StringHandler
          required: true
      auth:
        required: true
        plugins: ["ecpds"]
  ```

Then:

```bash
ECPDS_ENABLED=true \
  ECPDS_EVENT_TYPE=ecpds_test \
  ECPDS_MATCH_KEY=destination \
  ECPDS_ALLOWED_USER="<auth-o-tron-username>" \
  ECPDS_ALLOWED_PASS="<auth-o-tron-password>" \
  ECPDS_ALLOWED_DESTINATION="<destination-the-user-can-read>" \
  ECPDS_DENIED_DESTINATION="NOT-A-REAL-DEST" \
  AUTH_ADMIN_USER=admin-user \
  AUTH_ADMIN_PASS=admin-pass \
  python3 scripts/smoke_test.py
```

What the three ECPDS smoke cases verify:

| Case | What it asserts |
|---|---|
| `ecpds: allowed user + entitled destination -> 200` | `POST /api/v1/watch` returns HTTP 200 for `ECPDS_ALLOWED_USER` reading `ECPDS_ALLOWED_DESTINATION`. |
| `ecpds: allowed user + DENIED destination -> 403` | Same endpoint returns HTTP 403 for the same user reading `ECPDS_DENIED_DESTINATION`. |
| `ecpds: notify on ECPDS-protected stream is not gated` | `POST /api/v1/notification` returns 2xx for `AUTH_ADMIN_USER`. The plugin is read-only; a 503 here would mean it incorrectly ran on a write. |

Troubleshooting:

- All three skip with `[INFO] skipping ECPDS smoke test` → check `ECPDS_ENABLED=true` and that the required env vars are set.
- The allow case fails with `400` and a "schema validator before the plugin" hint: your `ECPDS_EVENT_TYPE` schema has additional required identifier fields. Add the minimal test schema above, or simplify the schema you're pointing at. The schema validator rejecting the request before ECPDS runs is the **correct** behaviour. The smoke test fails loudly here rather than papering over it.
- The allow case fails with `503` → the issue is between Aviso and ECPDS rather than at the plugin layer; see the [ECPDS Plugin Runbook](./ecpds-runbook.md).
- The notify-bypass case fails with `401`/`403` → your `AUTH_ADMIN_USER` / `AUTH_ADMIN_PASS` don't match your auth-o-tron config; that's an auth setup issue, not an ECPDS issue.

---

## Reporting a Problem

Every aviso response carries an `X-Request-ID` HTTP header and (for error
responses) a `request_id` field in the JSON body. Streaming responses also
include the same UUID in the first SSE event. When something goes wrong,
include this id in the bug report so the operator can find the matching
server logs in seconds. See [API Errors](./api-errors.md#how-to-report-a-problem)
for details.

## What's Next

- [Practical Examples](./practical-examples/overview.md): constraint filtering, spatial filtering, admin operations
- [Streaming Semantics](./streaming-semantics.md): full rules for watch/replay behavior, request id correlation, and reconnect protocol
- [Configuration Reference](./configuration-reference.md): all config fields and defaults
