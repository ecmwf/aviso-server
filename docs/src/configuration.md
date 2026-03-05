# Configuration

## Key Behaviors to Know First

Before diving into field-level settings, these rules affect how everything behaves at runtime:

- **Environment variables always win** — they override any YAML value regardless of which file it came from.
- **Replay and watch behavior is controlled by request parameters**, not static config switches.
- **Invalid policy values fail startup immediately** — `storage_type`, `retention_policy`, and `discard_policy` are parsed as typed enums; bad values are caught before any streams are created.
- **Per-schema `storage_policy` is validated at startup** against the selected backend's capabilities. Unsupported fields (e.g. `retention_time` on `in_memory`) cause a startup failure with a clear error.
- **JetStream stream changes are reconciled on access** — updating `compression`, retention, or limits in config takes effect when that stream is next accessed. Recreate the stream only if you need historical data physically rewritten.
- **`/api/v1/schema` responses are client-focused** — internal `storage_policy` settings are not exposed.

---

## Loading Precedence

Configuration is loaded in this order (later sources override earlier ones):

1. `./configuration/config.yaml`
2. `/etc/aviso_server/config.yaml`
3. `$HOME/.aviso_server/config.yaml`
4. Environment variables (highest precedence)

### Environment variable format

Prefix: `AVISOSERVER_`
Nested separator: `__`

```bash
AVISOSERVER_APPLICATION__HOST=0.0.0.0
AVISOSERVER_APPLICATION__PORT=8000
AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream
AVISOSERVER_NOTIFICATION_BACKEND__JETSTREAM__NATS_URL=nats://localhost:4222
```

---

## Config File Structure

The five top-level sections are:

| Section | Purpose |
|---|---|
| `application` | Server host, port, static files path |
| `logging` | Log level and format |
| `notification_backend` | Backend selection and backend-specific settings |
| `notification_schema` | Per-event-type validation, topic ordering, storage policy |
| `watch_endpoint` | SSE heartbeat, connection limits, replay batch settings |

`notification_backend.kind` selects the storage implementation:

- `jetstream` — production backend (NATS JetStream)
- `in_memory` — development backend (process-local, no persistence)

---

## Backend Details

- [Backends Overview](./backends-overview.md) — choose the right backend
- [In-Memory Backend](./backend-in-memory.md) — behavior and caveats
- [JetStream Backend](./backend-jetstream.md) — setup, stream management, operational notes
- Kubernetes deployment: [Helm chart](https://github.com/ecmwf/aviso-chart)

---

For full field-level documentation of every config option, see [Configuration Reference](./configuration-reference.md).
