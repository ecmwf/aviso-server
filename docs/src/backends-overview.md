# Backends Overview

Aviso server currently supports two backend kinds:

- `jetstream`
- `in_memory`

## Decision matrix

| Requirement | Recommended backend |
|---|---|
| Persistent history across restarts | `jetstream` |
| Replay endpoint support | `jetstream` |
| Live watch streaming support | `jetstream` |
| Multi-replica deployment | `jetstream` |
| Quick local experimentation with minimal setup | `in_memory` |

## Important current limitation

The current `in_memory` backend implementation has unimplemented streaming paths:

- `subscribe_to_topic` is `todo!`
- `get_messages_batch` is `todo!`

In practice, `watch`/`replay` endpoints require `jetstream` backend in current code.

See:

- [InMemory Backend](./backend-in-memory.md)
- [JetStream Backend](./backend-jetstream.md)

