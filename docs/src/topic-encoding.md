# Topic Encoding

Aviso now uses a single backend-agnostic wire format for topics across all backends.

## Why this exists

NATS subject tokenization is fixed to `.`. Wildcards (`*`, `>`) also operate on dot-delimited tokens.

If topic values are written directly to subjects, values containing reserved characters can break routing and filtering semantics.

Example:

- logical token value: `1.45`
- naive subject token: `1.45` (looks like two tokens to NATS)

To avoid this, Aviso encodes each token before building the wire subject.

## Invariants

- Wire subject separator is always `.`.
- Topic tokens are encoded/decoded with one shared codec for all backends.
- App-level wildcard matching is performed on decoded logical tokens.
- Decoder is strict: malformed `%HH` sequences are rejected.

## Encoding rules

Reserved characters are percent-encoded per token:

- `.` -> `%2E`
- `*` -> `%2A`
- `>` -> `%3E`
- `%` -> `%25`

`%` must be encoded to keep decoding unambiguous.

## Examples

- `1.45` -> `1%2E45`
- `1*34` -> `1%2A34`
- `1%25` -> `1%2525`

Roundtrip examples:

- `decode("1%2E45")` -> `1.45`
- `decode("1%2A34")` -> `1*34`
- `decode("1%2525")` -> `1%25`

Note:

- `decode("1%25")` -> `1%` (single decode pass)

## Impact on topic matching

Two-step matching still applies:

1. Backend coarse filter (JetStream subject filter).
2. App-level wildcard match.

Both steps are now safe with reserved characters because matching works on decoded logical tokens.

