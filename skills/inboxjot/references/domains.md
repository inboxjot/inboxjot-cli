# Domains

Docs: https://inboxjot.com/docs/domains, https://inboxjot.com/docs/api-reference/create-domain, https://inboxjot.com/docs/api-reference/verify-domain

You send only from a domain that is verified in the workspace. A send from another domain answers `403 forbidden`.

## Add a Domain

- Use a subdomain such as `send.acme.dev`. It keeps the sending reputation apart from the main domain.
- Pick the region when you add the domain. You cannot change it later.
- Only the user can add DNS records. An agent writes the code and tells the user which records to add.

API (Full key):

```bash
curl -X POST https://api.inboxjot.com/v1/domains \
  -H "Authorization: Bearer $INBOXJOT_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{ "name": "send.acme.dev" }'
```

| Field         | Rules                                                          |
| ------------- | -------------------------------------------------------------- |
| `name`        | Required. Up to 253 characters, at least two labels.           |
| `region`      | Optional, such as `us-east-1`. Defaults to the default region. |
| `return_path` | Optional subdomain for bounces. Defaults to `send`.            |

`409 conflict` means the domain is already in this workspace or another one.

## DNS Records

Copy the exact values from the dashboard or the API response. The shapes are:

| Record             | Name                  | Value                            |
| ------------------ | --------------------- | -------------------------------- |
| TXT, DKIM          | `inboxjot._domainkey` | `v=DKIM1; k=rsa; p=<public key>` |
| CNAME, return path | `send`                | `send-<region>.inboxjot-dns.com` |
| TXT, DMARC         | `_dmarc`              | `v=DMARC1; p=none;`              |

- The DKIM value is longer than 255 characters. Most DNS providers split it for you.
- DMARC is recommended but not needed for verification. Put it on the root domain, such as `_dmarc.acme.dev`. It covers the subdomains.

## Verification

- The domain is verified when the DKIM and return path records are found.
- Inboxjot checks every 10 minutes. `POST /v1/domains/{domain_id}/verify` checks now.
- A domain not verified within 72 hours is deleted. Add it again when the records are ready.
- Statuses: `not_started`, `pending`, `verified`, `failed`, `temporary_failure`.
- `pending_record_kinds` on a domain lists the records not found yet.

| Call                                  | Use                                           |
| ------------------------------------- | --------------------------------------------- |
| `GET /v1/domains`                     | List, oldest first.                           |
| `GET /v1/domains/{domain_id}`         | One domain with its records and their status. |
| `POST /v1/domains/{domain_id}/verify` | Check now.                                    |
| `DELETE /v1/domains/{domain_id}`      | Remove. You can no longer send from it.       |

The MCP tools `list_domains` and `get_domain` show the same, and `get_domain` adds the last 30 days of sends and bounces.

## Domain Settings

On a verified domain, in the dashboard:

- **Track opens** and **Track clicks** are off by default.
- **TLS** sets whether delivery needs an encrypted connection.
- **Warmup** ramps up campaign and automation volume for 14 days. It is on by default. API and SMTP email is never held.
- One verified domain can be the workspace default.
- The return path domain can change.

For SPF, DKIM, DMARC and warm-up practice, use the `sending-standards` skill.
