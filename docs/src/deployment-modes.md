# Deployment Modes

## Local experimentation

Recommended backend:

- `in_memory` for quick local request/validation testing.

Characteristics:

- No persistence: data is lost on process restart.
- Single-process state only.
- Not suitable for horizontal scaling or replica failover.
- Supports replay and watch in-process, limited by local memory retention.
- For local JetStream testing, use `./scripts/init_nats.sh` and point app config to `nats://localhost:4222`.
- You can run `python3 scripts/smoke_test.py` for a quick end-to-end behavior check.

## Production-like / persistent mode

Recommended backend:

- `jetstream`

Characteristics:

- Durable message storage.
- Retention and size limits.
- Replica support (requires clustered NATS setup).
- Supports replay and live streaming workflows.

Recommended packaging/deployment for Kubernetes:

- Helm chart: <https://github.com/ecmwf/aviso-chart>

## Selection guideline

- Need persistence/replay/streaming robustness: use `jetstream`.
- Need fastest setup for local functional checks only: use `in_memory` (node-local replay/watch).
