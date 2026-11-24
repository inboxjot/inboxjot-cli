# Campaigns and Automations

Docs: https://inboxjot.com/docs/campaigns, https://inboxjot.com/docs/automations, https://inboxjot.com/docs/api-reference/create-campaign, https://inboxjot.com/docs/api-reference/send-campaign

Campaigns send one email to segments. Automations send a series of emails to each contact who meets a trigger. Both are marketing email: they go only to subscribed contacts, they carry an unsubscribe link and one-click `List-Unsubscribe` headers, and domain warmup applies.

## Before the First Send

- The from address is on a verified domain.
- The postal address is set in Settings, or on each sending domain.
- The audience has at least one subscribed contact.

## Campaigns

### Lifecycle

`draft` → `scheduled` → `sending` → `sent`. A campaign can also be `paused`, `failed` or `canceled`.

| Call (Full key)                                        | Effect                                                                                                                                  |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /v1/campaigns`                                   | Creates a draft. Nothing is sent.                                                                                                       |
| `GET /v1/campaigns`, `GET /v1/campaigns/{campaign_id}` | List (filter `query`, `status`; sort `sent`, `name`) and read.                                                                          |
| `PATCH /v1/campaigns/{campaign_id}`                    | Changes a `draft` or `scheduled` campaign. Otherwise `409 conflict`. The change shows live to people who have it open.                  |
| `POST /v1/campaigns/{campaign_id}/test`                | Sends to 1 to 5 addresses in `to`, with `[Test]` before the subject. Counts toward the daily limit.                                     |
| `POST /v1/campaigns/{campaign_id}/send`                | Sends now, or schedules with a future `scheduled_at`. Call again on a scheduled campaign to change the time or send now. Answers `202`. |
| `POST /v1/campaigns/{campaign_id}/cancel`              | Cancels a scheduled, sending, paused or failed campaign. Sent emails stay sent.                                                         |
| `DELETE /v1/campaigns/{campaign_id}`                   | Deletes a draft or canceled campaign.                                                                                                   |

Create and update fields:

| Field                                        | Rules                                                                           |
| -------------------------------------------- | ------------------------------------------------------------------------------- |
| `name`                                       | 1 to 200 characters.                                                            |
| `from`                                       | Up to 320 characters. Must be on a verified domain before you send.             |
| `reply_to`                                   | `null` clears.                                                                  |
| `subject`                                    | Up to 998 characters.                                                           |
| `preheader`                                  | Up to 500 characters.                                                           |
| `html`                                       | Inboxjot turns it into editor content. `html` wins over `content`.              |
| `template_id`                                | Start from a template. `null` unlinks.                                          |
| `include_segment_ids`, `exclude_segment_ids` | Up to 50 each.                                                                  |
| `topic_id`                                   | Only contacts subscribed to it get the campaign. Defaults to the default topic. |

### Who Gets It

- Contacts in an included segment and not in an excluded segment.
- Only `subscribed` contacts that the topic allows.
- Suppressed addresses and paused contacts are skipped.

### Send Checks

Send Campaign checks the from domain, the subject, the content, the postal address and the audience. A failure answers `400 validation_error`. A scheduled campaign checks again when it starts, and goes back to draft if a check fails.

### Footer

Inboxjot adds your postal address and an unsubscribe link to the footer when the content does not show them. To place them yourself, use `{{mailing_address}}` and link to `{{unsubscribe_url}}`.

### Pauses

| Reason              | Resumes                                 |
| ------------------- | --------------------------------------- |
| Daily limit         | On its own after midnight UTC.          |
| Warmup              | On its own when the domain cap allows.  |
| Sending paused      | When workspace sending is active again. |
| Domain not verified | When the domain is verified again.      |

`paused_reason` on the campaign says why.

## Automations

Automations are built and turned on in the dashboard. The API has no automation routes. Your code feeds them through contacts, topics, segments and `POST /v1/events`. The MCP tool `list_automations` lists them with their trigger and status.

| Trigger             | Starts when                                              |
| ------------------- | -------------------------------------------------------- |
| Event received      | Your app sends the event with `POST /v1/events`.         |
| Subscribed to topic | A contact subscribes to the topic.                       |
| Added to segment    | A contact is added to a static segment.                  |
| Matches segment     | A contact matches a dynamic segment. Checked every hour. |

| Step           | Does                                               |
| -------------- | -------------------------------------------------- |
| Send Email     | Sends a template. The from and subject can change. |
| Wait           | 1 to 365 minutes, hours or days.                   |
| Wait for Event | Waits for an event from the contact, or a timeout. |
| Check If       | Splits into Yes and No paths.                      |

- A contact goes through once by default. Allow re-entry lets them go again after they finish. Matches segment always lets each contact through once.
- Up to 100 steps.
- Turn the automation off to change the trigger or steps. Runs pause while it is off.
- A run exits when the contact unsubscribes, is suppressed or is deleted.

## Over MCP

`create_campaign` makes a draft only. `send_test_campaign` sends a test to up to 5 addresses. There is no MCP tool that sends or schedules a campaign. The user does that in the dashboard, or code calls Send Campaign. See [mcp.md](mcp.md) for editing live content.
