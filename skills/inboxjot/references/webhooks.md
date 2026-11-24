# Webhooks

Docs: https://inboxjot.com/docs/webhooks, https://inboxjot.com/docs/api-reference/event-types, https://inboxjot.com/docs/api-reference/verify-signatures, https://inboxjot.com/docs/api-reference/create-webhook

A webhook sends a `POST` to your server when something happens to an email or a contact. Use it instead of polling `GET /v1/emails/{email_id}`, which needs a Full key.

## Create an Endpoint

In the dashboard (Webhooks › Add Endpoint) or with the API (Full key):

```bash
curl -X POST https://api.inboxjot.com/v1/webhooks \
  -H "Authorization: Bearer $INBOXJOT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Production",
    "url": "https://acme.dev/webhooks/inboxjot",
    "events": ["email.delivered", "email.bounced", "email.complained"]
  }'
```

| Field    | Rules                                                                             |
| -------- | --------------------------------------------------------------------------------- |
| `name`   | 1 to 100 characters.                                                              |
| `url`    | `https://` on a public address, up to 2,000 characters, no user name or password. |
| `events` | At least one event type. On update, the list replaces the old one.                |
| `status` | Update only: `enabled` or `disabled`. Send `enabled` to turn an endpoint back on. |

**The response holds the signing secret once.** Store it as `INBOXJOT_WEBHOOK_SECRET`. Later, copy it from the webhook page in the dashboard. The API and the MCP tool `list_webhooks` never return it again.

Other calls: `GET /v1/webhooks`, `GET /v1/webhooks/{endpoint_id}` (with recent deliveries), `PATCH /v1/webhooks/{endpoint_id}`, `DELETE /v1/webhooks/{endpoint_id}`.

## Event Types

| Event                    | Sent when                                                                                                                                            |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `email.sent`             | Inboxjot handed the email to the mail server.                                                                                                        |
| `email.delivered`        | The receiving server accepted it.                                                                                                                    |
| `email.delivery_delayed` | Delivery is delayed and still retrying.                                                                                                              |
| `email.opened`           | Opened. `actor`: `human`, `apple_mpp`, `scanner` or `unknown`.                                                                                       |
| `email.clicked`          | A link was clicked. `actor`: `human`, `scanner` or `unknown`. `link` when known.                                                                     |
| `email.bounced`          | Refused. `bounce.type`: `permanent`, `transient` or `undetermined`. `bounce.message` is the server reply. A permanent bounce suppresses the address. |
| `email.complained`       | Marked as spam. `complaint.type` is `abuse`.                                                                                                         |
| `email.rejected`         | The mail server refused to send it. `reason`.                                                                                                        |
| `email.failed`           | It could not be sent. `reason`.                                                                                                                      |
| `email.unsubscribed`     | The recipient unsubscribed.                                                                                                                          |
| `contact.created`        | A contact was created.                                                                                                                               |
| `contact.updated`        | A contact changed.                                                                                                                                   |
| `contact.deleted`        | A contact was deleted. Data is the contact as it was.                                                                                                |

Email event data: `email_id`, `to`, `from`, `subject`, and `tags` when the email had tags. Contact event data: `contact_id`, `email`, `first_name`, `last_name`, `status`, `language`, `properties`, `created_at`, `updated_at`.

```json
{
  "id": "evt_…",
  "type": "email.delivered",
  "created_at": "2026-10-17T09:30:00.000Z",
  "data": {
    "email_id": "eml_…",
    "to": ["riti@example.com"],
    "from": "Acme <hello@send.acme.dev>",
    "subject": "Your receipt from Acme"
  }
}
```

## Verify Signatures

Requests follow the Standard Webhooks spec.

| Header              | Value                                           |
| ------------------- | ----------------------------------------------- |
| `webhook-id`        | The event id, the same as `id` in the body.     |
| `webhook-timestamp` | Unix time in seconds.                           |
| `webhook-signature` | One or more `v1,<base64>`, separated by spaces. |

1. Read the **raw** body. Do not parse and re-encode it.
2. Remove the `whsec_` prefix from the secret and base64-decode the rest.
3. Compute HMAC-SHA256 over `{webhook-id}.{webhook-timestamp}.{body}` with that key. Base64-encode the result.
4. Accept when one signature in the header matches. Use a constant-time compare.
5. Refuse a timestamp more than 5 minutes old.

The SDKs do this: `verifyWebhook(body, headers, secret)` in Node.js, `verify_webhook(body, headers, secret)` in Python, `webhooks::verify(&secret, &headers, &body)` in Rust, `webhook.Verify(payload, r.Header, secret)` in Go.

```javascript
import { verifyWebhook } from 'inboxjot';

export async function POST(request) {
  const event = await verifyWebhook(
    await request.text(),
    request.headers,
    process.env.INBOXJOT_WEBHOOK_SECRET
  );
  await queue.add(event.id, event);
  return new Response(null, { status: 204 });
}
```

## Delivery and Retries

- Answer `2xx` within 10 seconds. Queue slow work.
- A failed delivery retries 5 times: after 1 minute, 10 minutes, 1 hour, 6 hours and 16 hours.
- The `webhook-id` stays the same across retries. Store it and skip ids you already handled.
- If the last retry fails and the endpoint had no successful delivery in the last 24 hours, Inboxjot turns the endpoint off. Turn it on with `PATCH` and `status: "enabled"`.
- The dashboard Event Log lists every delivery. Replay Event sends one again. Send Test Event checks your setup.
