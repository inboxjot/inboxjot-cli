# Contacts, Segments, Topics and Events

Docs: https://inboxjot.com/docs/contacts, https://inboxjot.com/docs/segments, https://inboxjot.com/docs/topics, https://inboxjot.com/docs/track-events

Contacts are the people who get marketing email: campaigns and automations. Transactional email through the API does not need a contact. Every route here needs a **Full** key. A key limited to one domain answers `403`.

## Contacts

### Create or Update

`POST /v1/contacts` creates the contact, or updates the contact with the same address. It answers `201` for a new contact and `200` for an existing one. The response has `created: true` for a new contact.

| Field                     | Rules                                                                                                                                                                                 |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `email`                   | Required. Up to 320 characters. Invalid and disposable addresses are refused.                                                                                                         |
| `first_name`, `last_name` | Up to 200 characters. `null` clears.                                                                                                                                                  |
| `language`                | A code such as `en` or `pt-BR`.                                                                                                                                                       |
| `notes`                   | Up to 5,000 characters. `null` clears.                                                                                                                                                |
| `properties`              | Values keyed by property key. Each value matches its type: text up to 2,000 characters, number, boolean, date, or a choice option. `null` clears one. Values merge with the old ones. |
| `subscribed`              | A new contact is subscribed. `false` unsubscribes from all email. Leave it out to keep an existing contact's status.                                                                  |
| `segment_ids`             | Up to 50 static segments to add. Each must exist.                                                                                                                                     |

### Read, Update, Delete

`GET`, `PATCH` and `DELETE /v1/contacts/{contact_id}` take the contact id **or** the email address.

- `GET` returns the contact with its segments, every topic and every brand with the contact's choice.
- `PATCH` updates only the fields you send. It also takes `topics`: up to 100 `{ "id", "status" }` with `subscribed` or `unsubscribed`, and `brands`: up to 20 in the same shape. Unsubscribing from a brand stops every topic of that brand.
- `PATCH` with `segment_ids` **replaces** the static segments. The contact leaves the ones not in the list.
- `DELETE` removes the contact. For a privacy request, use `POST /v1/contacts/erase` with the `email`: Inboxjot also deletes their emails, events and logs in the workspace, and keeps the address on the suppression list.

### List

`GET /v1/contacts`, newest first, with an exact `total`. Filters: `query` (address or name, up to 200 characters), `status`, `segment_id`. Sort: `added`, `contact`, `last_activity`.

### Statuses

| Status         | Meaning                                                       |
| -------------- | ------------------------------------------------------------- |
| `subscribed`   | Gets campaigns and automation email.                          |
| `pending`      | Waits for double opt-in confirmation. Shown as Not Confirmed. |
| `unsubscribed` | Gets no marketing email.                                      |

Bounces and complaints do not change the status. They add the address to the suppression list. `is_suppressed` shows it in the list.

### Properties

Define custom properties in the dashboard (Contacts › Properties) before you send them. Types: Text, Number, Date, True or false, Choice. A key starts with a letter and uses lowercase letters, numbers and underscores. The key cannot change. The MCP tool `list_contact_properties` lists them.

### CSV Import

Dashboard only. Up to 100 MB. One column must be the email. To subscribe the contacts to a topic, you confirm they gave consent. Without a topic, new contacts are saved as unsubscribed.

## Segments

A segment is a group of contacts. Campaigns send to segments. The type cannot change after creation.

| Type      | Members                                                                                       |
| --------- | --------------------------------------------------------------------------------------------- |
| `static`  | The contacts you add: by hand, by CSV import, with `segment_ids`, or with the members routes. |
| `dynamic` | Every contact that matches its conditions. It updates as contacts change.                     |

| Call                                                | Use                                                                                                         |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `POST /v1/segments`                                 | `name` (1 to 100), `type`, optional `description` (up to 500), and `conditions` for a dynamic segment only. |
| `GET /v1/segments`, `GET /v1/segments/{segment_id}` | List (filter by `type`, `query`) and read.                                                                  |
| `PATCH /v1/segments/{segment_id}`                   | New `conditions` replace the old ones.                                                                      |
| `DELETE /v1/segments/{segment_id}`                  | The contacts stay.                                                                                          |
| `GET /v1/segments/{segment_id}/members`             | Static only. For a dynamic segment, use `GET /v1/contacts?segment_id=…`.                                    |
| `POST` / `DELETE /v1/segments/{segment_id}/members` | `contact_ids`, 1 to 1,000. Static only.                                                                     |

Conditions:

```json
{
  "match": "all",
  "rules": [
    { "field": "property.plan", "operator": "is", "value": "pro" },
    { "field": "email.opened", "operator": "within", "value": "30d" }
  ]
}
```

- `match`: `all` or `any`. Up to 20 rules.
- Fields: `email`, `first_name`, `last_name`, `language`, `status`, `created_at`, `segment`, `email.sent`, `email.delivered`, `email.opened`, `email.clicked`, `email.bounced`, `email.complained`, `property.<key>`.
- Text operators: `is`, `is_not`, `contains`, `not_contains`, `starts_with`, `ends_with`, `is_set`, `is_not_set`.
- Number operators: `equals`, `not_equals`, `greater_than`, `less_than`.
- Date operators: `within`, `not_within`, `before`, `after`. A window is a date or a number of days, such as `30d`.
- `segment`: `in`, `not_in`, with a static segment id.
- Email activity: `within`, `not_within`, `at_least`, `never`. `at_least` takes `{ "count": 2, "days": 30 }`, count up to 1,000, up to 400 days.

## Topics

A topic is a kind of email a contact can choose, such as product news. Every campaign sends on one topic. Each workspace starts with a default Marketing topic. A workspace can have up to 100 topics (`403 limit_exceeded` after that).

A contact gets a topic when their topic status is `subscribed`, or when they have no status and the topic is on by default.

| Field (`POST /v1/topics`, `PATCH /v1/topics/{topic_id}`) | Rules                                                                      |
| -------------------------------------------------------- | -------------------------------------------------------------------------- |
| `name`                                                   | 1 to 100 characters. Required on create.                                   |
| `description`                                            | Up to 500 characters. Shown on the preference page.                        |
| `default_subscribed`                                     | New contacts get the topic until they choose.                              |
| `visibility`                                             | `public` shows it on the preference page. `private` hides it.              |
| `double_opt_in`                                          | A form sign-up must confirm by email first.                                |
| `form_enabled`                                           | Turns on the hosted sign-up form. Needs a sender first.                    |
| `sender_name`, `sender_email`                            | From for confirmation emails. `sender_email` is on a verified domain.      |
| `redirect_url`                                           | An `https://` page after sign-up.                                          |
| `brand_id`                                               | The brand that owns the topic. Defaults to the default brand. Create only. |

- `DELETE /v1/topics/{topic_id}` moves its campaigns to the default topic. The default topic cannot be deleted.
- **The API cannot subscribe a contact to a topic they unsubscribed from.** Only the contact can, on the preference page.

## Brands

A brand is a name readers see on the unsubscribe page and in email footers, with its own logo and topics. Each workspace starts with a default brand. A workspace can have up to 20 brands. A contact who leaves one brand stays subscribed to the others.

- `GET /v1/brands` lists brands, oldest first. `POST /v1/brands` and `PATCH /v1/brands/{brand_id}` take `name` (1 to 100 characters).
- `DELETE /v1/brands/{brand_id}` fails for the default brand and for a brand that still has topics.
- Add a logo in the dashboard. The API does not upload logos.
- **The API cannot subscribe a contact to a brand they unsubscribed from.** Only the contact can.

### Sign-Up Form

A public topic with the form on accepts `POST /v1/forms/{topic_id}` with `email` and optional `first_name` and `last_name`. With double opt-in, the confirmation link works for 7 days.

```html
<form method="POST" action="https://api.inboxjot.com/v1/forms/<topic_id>">
  <input type="email" name="email" required />
  <button type="submit">Subscribe</button>
</form>
```

## Track Events

`POST /v1/events` tells Inboxjot a contact did something. It answers `202` with `contact_id`.

| Field   | Rules                                                                                                       |
| ------- | ----------------------------------------------------------------------------------------------------------- |
| `name`  | Required. 1 to 100 characters. Starts with a letter or number, then letters, numbers, `.`, `_`, `:` or `-`. |
| `email` | Required. A new address creates a **subscribed** contact. Disposable addresses are refused.                 |
| `data`  | Optional JSON object, up to 16 KB.                                                                          |

- Automations with an Event received trigger for this name start a run.
- Runs that wait for this event continue.
- Emails in an automation started by the event can use `{{event.<field>}}` for each top-level field of `data`. Data from an event that resumes a Wait for Event step is not added.
- Inboxjot sets the event time.

## MCP Tools

`list_contacts`, `get_contact`, `create_contact`, `update_contact`, `delete_contact`, `list_segments`, `list_topics`, `list_contact_properties`. There is no MCP tool for events or for creating segments and topics.
