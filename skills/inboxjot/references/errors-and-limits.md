# Errors, Limits and Idempotency

Docs: https://inboxjot.com/docs/errors, https://inboxjot.com/docs/api-reference/errors, https://inboxjot.com/docs/api-reference/rate-limits, https://inboxjot.com/docs/api-reference/idempotency, https://inboxjot.com/docs/sending-limits, https://inboxjot.com/docs/logs

## Error Body

Every error has the same JSON body:

```json
{
  "name": "validation_error",
  "status": 400,
  "message": "Either html or text is required",
  "errors": [
    { "message": "Either html or text is required", "location": "html" }
  ],
  "request_id": "…"
}
```

- Branch on `name`. It is stable. `message` is for people and can change.
- `errors[].location` is a dot path, such as `attachments.0.filename`. It is `body` when the body is not valid JSON.
- `request_id` matches the `X-Request-Id` header. Search for it in Logs.

## Error Names

| Status | Name                             | Cause                                                                                                            | Retry                    |
| ------ | -------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------ |
| 400    | `validation_error`               | A field is missing or invalid, the body is not JSON, or an attachment could not be read.                         | No                       |
| 400    | `invalid_idempotency_key`        | The key is not 1 to 256 printable ASCII characters.                                                              | No                       |
| 401    | `unauthorized`                   | The key is missing, revoked or expired.                                                                          | No                       |
| 403    | `forbidden`                      | The key cannot do this, or the from domain is not verified, every recipient is suppressed, or sending is paused. | No                       |
| 403    | `limit_exceeded`                 | A workspace limit, such as 100 topics.                                                                           | No                       |
| 404    | `not_found`                      | The object is not in this workspace.                                                                             | No                       |
| 409    | `conflict`                       | The object is in the wrong state, or the name is taken.                                                          | No                       |
| 409    | `concurrent_idempotent_requests` | A request with this key is still running.                                                                        | Yes, after 1 second      |
| 413    | `payload_too_large`              | Over 45 MB for Send Email, 10 MB for Send Batch, 2 MB for other routes.                                          | No                       |
| 422    | `invalid_idempotent_request`     | The key was used with a different request.                                                                       | No                       |
| 429    | `rate_limit_exceeded`            | Over 10 requests per second for the key, or the daily sending limit.                                             | Yes, after `Retry-After` |
| 500    | `unknown_error`                  | A failure on the Inboxjot side.                                                                                  | Yes, with backoff        |

## Retry Loop

1. Send with an `Idempotency-Key`.
2. On `429`, `409` or `5xx`, wait for `Retry-After` (or `RateLimit-Reset`, or a short backoff when neither is sent). Retry with the **same** key.
3. On any other `4xx`, stop. Surface the `message`.
4. Cap the attempts.

## Rate Limit

Each API key can make 10 requests per second. SMTP shares the counter.

| Header                | Meaning                                 |
| --------------------- | --------------------------------------- |
| `RateLimit-Policy`    | The limit and window, such as `10;w=1`. |
| `RateLimit-Limit`     | Requests allowed in each window.        |
| `RateLimit-Remaining` | Requests left in this window.           |
| `RateLimit-Reset`     | Seconds until the window resets.        |
| `Retry-After`         | Seconds to wait. Only on a `429`.       |

## Daily Sending Limit

- A new workspace can send 100 emails a day.
- Each recipient counts once. One email to three addresses across `to`, `cc` and `bcc` counts as three.
- The count resets at midnight UTC.
- Over the limit, sends answer `429 rate_limit_exceeded`. A batch is accepted only when the whole batch fits.
- Campaign test sends count too.
- Settings › Sending shows today's count. Request Increase asks for more.

## Sending Pauses

Inboxjot pauses sending when the bounce rate reaches 4% or the complaint rate reaches 0.08%. While paused, sends answer `403 forbidden`. A pause for a high rate needs support to resume.

## Idempotency

Every route that changes data takes an optional `Idempotency-Key` header.

- 1 to 256 printable ASCII characters. Scoped to the workspace. Kept for 24 hours.
- Inboxjot compares the method, the path and the body.
- The same request replays the stored response with `Idempotent-Replayed: true`. The email is sent once.
- A different request with the same key answers `422 invalid_idempotent_request`.
- A request still running answers `409 concurrent_idempotent_requests` with `Retry-After: 1`.
- Only a `2xx` is stored. After any other answer, fix the request and retry with the same key.
- A request cut off without a response releases its key after 60 seconds. A retry then runs again.
- SMTP has no idempotency keys.

Make the key from stable data: `order-1042-receipt`, `user-881-welcome`. Never generate a new random key per attempt.

SDK forms: Node.js `inboxjot.emails.send(email, { idempotencyKey })`, Python `inboxjot.emails.send(email, idempotency_key=…)`, Go `inboxjot.WithIdempotencyKey(…)`, Rust `client.emails().idempotency_key(…).send(&email)`.

## Logs

The dashboard Logs page lists every API request made with an API key: method, path, status, duration, key and IP. The detail shows the request and response bodies, with the key hidden.

- MCP tool calls are logged. Dashboard actions, SMTP sessions and requests with an invalid key are not.
- Filter by All, Succeeded or Failed, or by API key. Search by endpoint or request id.
- Logs are kept for 90 days. Export Logs saves a CSV file.
