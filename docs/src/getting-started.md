# Getting Started

This guide walks you through running Aviso Server locally and sending your first notification.
It assumes you have already completed [Installation](./installation.md).

If you haven't read [Key Concepts](./concepts.md) yet, do that first —
it will make the commands below much easier to follow.

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
This is a live SSE stream — keep it open while you proceed to step 6.

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
data: {"type":"connection_established","timestamp":"2026-03-04T10:00:00Z"}
```

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
{ "id": "my_event@1", "topic": "my_event.north.20250706" }
```

The `id` (`my_event@1`) is the backend sequence reference.
You can use it later to replay from that point or delete that specific notification.

Switch back to the watch terminal — the notification should have arrived:

```
data: {"specversion":"1.0","id":"my_event@1","type":"aviso.notification",...}
```

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
data: {"type":"connection_closing","reason":"end_of_stream","timestamp":"..."}
```

---

## Optional: Local JetStream Setup

To test with the JetStream backend (persistent storage, more realistic), see
[Installation — Local JetStream](./installation.md#local-jetstream-docker) for the full setup
including environment variables and token authentication.

---

## Run the Smoke Test

A Python smoke script covers the full notify → watch → replay cycle.
Copy the example config and start the server:

```bash
cp configuration/config.yaml.example configuration/config.yaml
cargo run
```

**With auth (default)** — start auth-o-tron before running the smoke tests:

```bash
python3 -m pip install httpx
./scripts/auth-o-tron-docker.sh
python3 scripts/smoke_test.py
```

**Without auth** — set `auth.enabled: false` in your config (or remove the `auth` section), then:

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
- (optional, off by default) ECPDS plugin allow/deny/notify-bypass — see below

### Optional: end-to-end ECPDS plugin smoke test

If your build has `--features ecpds` enabled and your config has a working `ecpds:` block pointing at your real ECPDS servers, the smoke script can verify the plugin end-to-end against that ECPDS. It's off by default.

You need a known **allowed** user (one entitled to a specific destination per your ECPDS) and a destination value that user is **not** entitled to. Then:

```bash
ECPDS_ENABLED=true \
  ECPDS_EVENT_TYPE=dissemination \
  ECPDS_MATCH_KEY=destination \
  ECPDS_ALLOWED_USER="<jwt-username>" \
  ECPDS_ALLOWED_PASS="<password>" \
  ECPDS_ALLOWED_DESTINATION="<destination-the-user-can-read>" \
  ECPDS_DENIED_DESTINATION="NOT-A-REAL-DEST" \
  ECPDS_EXTRA_IDENTIFIER='{"class":"od"}' \
  python3 scripts/smoke_test.py
```

What the three ECPDS smoke cases verify:

| Case | What it asserts |
|---|---|
| `ecpds: allowed user + entitled destination -> 200` | The watch endpoint returns 200 for `ECPDS_ALLOWED_USER` reading `ECPDS_ALLOWED_DESTINATION`. |
| `ecpds: allowed user + DENIED destination -> 403` | The same user reading `ECPDS_DENIED_DESTINATION` is rejected with 403. |
| `ecpds: notify on ECPDS-protected stream is not gated` | An admin POSTing a notification on the ECPDS-protected stream does not get a 503 (the plugin is read-only). |

Tip: if all three skip with `[INFO] skipping ECPDS smoke test`, double-check `ECPDS_ENABLED=true` and that the required env vars are set.

If the first case fails with `503 Service Unavailable`, the issue is likely between Aviso and ECPDS rather than at the plugin layer — see the [ECPDS Plugin Runbook](./ecpds-runbook.md) for triage.

---

## What's Next

- [Practical Examples](./practical-examples/overview.md) — constraint filtering, spatial filtering, admin operations
- [Streaming Semantics](./streaming-semantics.md) — full rules for watch/replay behavior
- [Configuration Reference](./configuration-reference.md) — all config fields and defaults
