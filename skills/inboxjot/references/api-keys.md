# API Keys

Docs: https://inboxjot.com/docs/api-keys and https://inboxjot.com/docs/api-reference/authentication

An API key lets code call the API and the SMTP server. Send it as a bearer token:

```
Authorization: Bearer ibx_…
```

- Keys start with `ibx_`. Inboxjot stores only a hash of the secret part.
- The key is shown once, when it is created. Store it as `INBOXJOT_API_KEY`.
- Workspace admins create keys in the dashboard under API Keys, or with `inboxjot api-keys create` after `inboxjot login` with the `api_keys:write` scope. An API key cannot create, roll or delete keys.

## Permissions

| Permission     | Can call                                                                                                                  |
| -------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Sending access | `POST /v1/emails`, `POST /v1/emails/batch` and SMTP. Every other route answers `403` with "This key can only send email". |
| Full access    | Every API route, unless the key is limited to one domain.                                                                 |

Use Sending access for app code that only sends. Use Full access for code that reads email status, manages contacts, or sets up domains and webhooks.

## Restrictions

| Option       | Effect                                                                                                                                                                                               | Set             |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| Domain       | The key sends only from one verified domain. It sees only that domain's emails and attachments, and answers `403` on Contacts, Events, Domains, Templates, Campaigns, Segments, Topics and Webhooks. | At creation     |
| Expires      | The key stops working after 30 days, 90 days or 1 year.                                                                                                                                              | At creation     |
| IP allowlist | Up to 50 IP addresses. Other addresses get `403`.                                                                                                                                                    | On the key page |

## Failures

| Status             | Cause                                                                                                                                   |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| `401 unauthorized` | The key is missing, wrong, expired or revoked.                                                                                          |
| `403 forbidden`    | A Sending key on a Full route, a domain-limited key on a workspace route or another domain, or a request from outside the IP allowlist. |

## Roll and Revoke

- **Roll** makes a new secret and keeps the settings. The old key stops immediately, in 1 hour or in 24 hours. Expire Now on the key page stops it early. Roll at once if the key leaked.
- **Revoke** deletes the key. Requests with it fail with `401` at once. It cannot be undone. Emails sent with the key stay.

## Rules for Agents

- Read the key from `INBOXJOT_API_KEY`. Add `INBOXJOT_API_KEY=` with an empty value to `.env.example`.
- Never ask the user to paste the key into the chat. Never print, log or commit it.
- MCP uses OAuth sign-in, not an API key. See [mcp.md](mcp.md).
