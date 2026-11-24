# Sending

Docs: https://inboxjot.com/docs/send-your-first-email, https://inboxjot.com/docs/api-reference/send-email, https://inboxjot.com/docs/api-reference/send-batch, https://inboxjot.com/docs/templates

## Send One Email

`POST /v1/emails` with a Sending or Full key. The answer is `202 Accepted` with `{ "id": "eml_…" }`.

| Field            | Rules                                                                                                                                                                                      |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `from`           | On a verified domain, such as `Acme <hello@send.acme.dev>`. 3 to 320 characters. Required unless the template sets one.                                                                    |
| `to`             | One address or a list. An address can carry a name: `Jane <jane@example.com>`. Up to 50 recipients across `to`, `cc` and `bcc`. Repeated addresses are sent once.                          |
| `cc`, `bcc`      | One address or a list.                                                                                                                                                                     |
| `reply_to`       | One address or a list. Defaults to the template's reply-to.                                                                                                                                |
| `subject`        | 1 to 998 characters. Required unless the template sets one.                                                                                                                                |
| `html`, `text`   | Send one or both, unless the template has content. Send both for the best result.                                                                                                          |
| `template`       | `{ "id": "tpl_…", "variables": { … } }`. See Templates below.                                                                                                                              |
| `headers`        | Up to 50. Allowed: `In-Reply-To`, `References`, `List-Unsubscribe`, `List-Unsubscribe-Post` and most `X-` headers. A few `X-` names are reserved. String values, no line breaks.           |
| `tags`           | Up to 10 `{ "name", "value" }` pairs, each 1 to 256 letters, numbers, `_` or `-`. Names are unique. `email_id`, `workspace_id` and `campaign_id` are reserved. Tags come back in webhooks. |
| `scheduled_at`   | ISO 8601 with an offset, in the future, up to 30 days ahead. Leave it out to send now.                                                                                                     |
| `attachments`    | Up to 20 files, 30 MB in total. The whole request can be up to 45 MB.                                                                                                                      |
| `open_tracking`  | `true` or `false`. Turns open tracking on or off for this email. Leave it out to use the domain setting. Opens are tracked only when the domain has a verified tracking domain.            |
| `click_tracking` | `true` or `false`. Turns click tracking on or off for this email. Leave it out to use the domain setting. Clicks are tracked only when the domain has a verified tracking domain.          |

Attachment fields:

| Field          | Rules                                                                     |
| -------------- | ------------------------------------------------------------------------- |
| `filename`     | Required. 1 to 255 characters, no slashes or line breaks.                 |
| `content`      | The file as base64. Send `content` or `path`, not both.                   |
| `path`         | An HTTPS URL. It must answer `200` without a redirect, within 15 seconds. |
| `content_type` | Optional MIME type. Inboxjot picks one from the file name.                |
| `content_id`   | For an inline image. Reference it in the HTML as `cid:<content_id>`.      |

## Send a Batch

`POST /v1/emails/batch` takes an array of 1 to 100 email objects, up to 10 MB in total. The answer is `202` with `{ "data": [{ "id" }, …] }` in the order you sent.

- Each email takes the Send Email fields **without** `scheduled_at` and `attachments`.
- Inboxjot checks every email first. When one fails, it queues none. The error message starts with the email number, such as "Email 2".
- The daily limit accepts a batch only when the whole batch fits.
- Send one `Idempotency-Key` for the whole batch.

Use a batch for many different emails at once, such as one per user. For one email to a segment, use a campaign.

## Templates

A template holds content you send many times. Create it in the dashboard, with `POST /v1/templates`, or with the MCP `create_template` tool.

```json
{
  "to": "riti@example.com",
  "template": {
    "id": "tpl_…",
    "variables": { "first_name": "Riti", "order_total": "$42.00" }
  }
}
```

- The template's from, reply-to, subject and content fill the fields you leave out.
- Fields in the request win over the template.
- The template content is used only when the request has no `html` and no `text`.
- Variables fill the subject, the preheader and the content.

### Variables

| Syntax                            | Result                                                        |
| --------------------------------- | ------------------------------------------------------------- |
| `{{first_name}}`                  | The value, or empty when missing.                             |
| `{{first_name fallback="there"}}` | The value, or `there` when missing.                           |
| `{{unsubscribe_url}}`             | The preference page link. Campaign and automation email only. |
| `{{mailing_address}}`             | Your postal address. Campaign and automation email only.      |
| `{{event.order_id}}`              | A top-level field of the event that started an automation.    |

Contact properties use their key as the variable name. For an API send, pass every value in `template.variables`. The `email-design` skill covers the variable syntax. In the dashboard editor, set the fallback on the variable.

Create template body: `name` (required, 1 to 200), `from`, `reply_to`, `subject` (up to 998), `preheader` (up to 500), and `html` or `starter_id`. Starters: `welcome`, `verify-email`, `password-reset`, `receipt`, `newsletter`, `product-launch`, `event-invite`, `re-engagement`. `html` wins over `starter_id`. Inboxjot turns the HTML into editor content, so the template stays editable.

## Schedule, Reschedule, Cancel

| Call                                              | Effect                                                            |
| ------------------------------------------------- | ----------------------------------------------------------------- |
| `scheduled_at` on Send Email                      | Sends later, up to 30 days ahead.                                 |
| `PATCH /v1/emails/{email_id}` with `scheduled_at` | Moves a `scheduled` email. Full key.                              |
| `POST /v1/emails/{email_id}/cancel`               | Cancels a `scheduled` email. Status becomes `canceled`. Full key. |

## Look Up Emails

These need a **Full** key.

| Call                                                    | Use                                                                                                                                                   |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /v1/emails`                                        | Newest first. Filters: `query`, `status`, `status_op` (`is`, `not`), `source`, `sent_after`, `sent_before`. Sort: `sort` (`sent`, `to`), `direction`. |
| `GET /v1/emails/{email_id}`                             | One email and its latest status.                                                                                                                      |
| `GET /v1/emails/{email_id}/attachments`                 | Attachments, each with a `download_url` that works for 10 minutes.                                                                                    |
| `GET /v1/emails/{email_id}/attachments/{attachment_id}` | One attachment.                                                                                                                                       |

Email statuses: `scheduled`, `queued`, `sending`, `sent`, `delivered`, `opened`, `clicked`, `soft_bounced`, `bounced`, `rejected`, `failed`, `canceled`, `complained`.

Sources: `api`, `smtp`, `dashboard`, `mcp`, `campaign`, `automation`.

## Paging

Every list uses cursors. Read `next_cursor` and send it as `after`. Send `prev_cursor` as `before` to go back. `per_page` is 1 to 100, default 50. Never page with offsets.

## Ids

Ids carry a type prefix, such as `eml_` (email), `con_` (contact), `tpl_` (template) and `att_` (attachment). Ids sort by creation time.
