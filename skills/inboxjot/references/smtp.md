# SMTP

Docs: https://inboxjot.com/docs/smtp

Use SMTP when the app or tool cannot call the API. Each message goes through the same checks, limits and logs as `POST /v1/emails`. SMTP has **no templates and no idempotency keys**. Use the API when you need them.

## Connection

| Setting  | Value                                       |
| -------- | ------------------------------------------- |
| Host     | `smtp.inboxjot.com`                         |
| Port     | 465 with SSL, or 587 and 2525 with STARTTLS |
| Username | `inboxjot`                                  |
| Password | An API key with Sending access              |
| Auth     | PLAIN or LOGIN, after TLS starts            |

```javascript
import nodemailer from 'nodemailer';

const transport = nodemailer.createTransport({
  host: 'smtp.inboxjot.com',
  port: 465,
  secure: true,
  auth: { user: 'inboxjot', pass: process.env.INBOXJOT_API_KEY },
});
```

## How Messages Map

- To and Cc keep the header addresses that are also envelope recipients. Other envelope recipients become Bcc.
- The From header sets the sender. Without it, the `MAIL FROM` address is used.
- Reply-To, the subject, the HTML and text parts and attachments carry over.
- `X-` headers pass through, except a few reserved ones. `In-Reply-To`, `References` and the `List-Unsubscribe` headers pass through.
- Open and click tracking follow the sending domain settings.
- The email shows `smtp` as its source in Emails. SMTP sessions do not show in the API request Logs.

## Limits and Replies

| Limit                  | Value                                    |
| ---------------------- | ---------------------------------------- |
| Message size           | 40 MB                                    |
| Recipients per message | 50                                       |
| Failed sign-ins        | 3, then the server closes the connection |
| Idle timeout           | 5 minutes                                |

| Reply                            | Meaning                                                    |
| -------------------------------- | ---------------------------------------------------------- |
| `250 2.0.0 Queued as <email id>` | Accepted.                                                  |
| `451`                            | Temporary, such as a rate limit. The client retries later. |
| `550`                            | Auth or permission problem.                                |
| `554`                            | Invalid message.                                           |

The rate limit counter is shared with the API key (10 requests per second).
