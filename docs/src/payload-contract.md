# Payload Contract

This page defines the canonical payload behavior for Aviso notifications.

## Scope

- Applies to `POST /api/v1/notification` input.
- Applies to stored backend payload representation.
- Applies to replay/watch CloudEvent output payload field.

## Schema Configuration

Per event schema, payload configuration is:

```yaml
payload:
  required: true # or false
```

There is no `payload.type` list.

## Canonical Rules

1. Payload values are JSON values:
`object`, `array`, `string`, `number`, `boolean`, `null`.
2. If `payload.required = true` and request omits `payload`, request is rejected (`400`).
3. If `payload.required = false` and request omits `payload`, Aviso stores canonical JSON `null`.
4. Aviso does not wrap or reshape payload values (for example no auto-wrapping into `{"data": ...}`).

## Input to Storage to Replay Mapping

| Notify request payload | Stored payload | Replay/Watch CloudEvent `data.payload` |
|---|---|---|
| omitted (optional schema) | `null` | `null` |
| `"forecast complete"` | `"forecast complete"` | `"forecast complete"` |
| `42` | `42` | `42` |
| `true` | `true` | `true` |
| `["a","b"]` | `["a","b"]` | `["a","b"]` |
| `{"note":"ok"}` | `{"note":"ok"}` | `{"note":"ok"}` |

## Failure Cases

- Missing required payload:
  - HTTP `400`
  - validation error (`INVALID_NOTIFICATION_REQUEST`)
- Malformed JSON request body:
  - HTTP `400`
  - parse error (`INVALID_JSON`)

## Consumer Guidance

- Treat `data.payload` as dynamic JSON.
- If your client requires object-only payloads, normalize on the client side.
  - Example strategy: for non-object payloads, convert to `{"data": <payload>}` in your consumer.
