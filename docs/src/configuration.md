# Configuration

Configuration is loaded in this precedence order (later overrides earlier):

1. `./configuration/config.yaml`
2. `/etc/aviso_server/config.yaml`
3. `$HOME/.aviso_server/config.yaml`
4. environment variables (highest precedence)

Environment variable prefix:

- `AVISOSERVER_`
- nested separator: `__`

Example:

```bash
AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream
AVISOSERVER_NOTIFICATION_BACKEND__JETSTREAM__NATS_URL=nats://localhost:4222
```

Common local JetStream block:

```yaml
notification_backend:
  kind: jetstream
  jetstream:
    nats_url: "nats://localhost:4222"
```

## Main sections

- `application`
- `logging`
- `notification_schema`
- `watch_endpoint`
- `notification_backend`

`notification_backend.kind` selects implementation:

- `jetstream`
- `in_memory`

Backend details:

- [Backends Overview](./backends-overview.md)
- [InMemory Backend](./backend-in-memory.md)
- [JetStream Backend](./backend-jetstream.md)
- Helm chart for Kubernetes deployment: <https://github.com/ecmwf/aviso-chart>

## Practical Notes

- Environment variables override YAML values.
- Replay/watch behavior is controlled by request parameters, not by static config switches.
- Invalid JetStream policy values (for example `storage_type`) are rejected during configuration deserialization at startup (fail-fast), before streams are created.
- Per-schema `storage_policy` is validated at startup against backend capabilities.
- `/api/v1/schema` responses remain client-focused and do not expose internal `storage_policy`.

Use [Configuration Reference](./configuration-reference.md) for full field-level documentation.
