# aviso-validators

Validation primitives used by `aviso-server`.

This crate contains reusable validation logic for identifier values and related
schema constraints.

Spatial validators use latitude-then-longitude JSON coordinates. Point and
polygon parsers retain their compatible string inputs. `PointCloudHandler`
accepts non-empty JSON arrays, preserves point order and duplicates, and applies
configurable count and serialized-size limits.

## Installation

```bash
cargo add aviso-validators
```

## License

Apache-2.0
