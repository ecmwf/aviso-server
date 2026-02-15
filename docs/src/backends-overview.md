# Backends Overview

Aviso server currently supports two backend kinds:

- `jetstream`
- `in_memory`

## Decision matrix

| Requirement | Recommended backend |
|---|---|
| Persistent history across restarts | `jetstream` |
| Replay endpoint support | `jetstream` (or `in_memory` for local/node-local use) |
| Live watch streaming support | `jetstream` (or `in_memory` for local/node-local use) |
| Multi-replica deployment | `jetstream` |
| Quick local experimentation with minimal setup | `in_memory` |

See:

- [InMemory Backend](./backend-in-memory.md)
- [JetStream Backend](./backend-jetstream.md)
