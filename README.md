![Inboxjot](https://inboxjot.com/og.png)

<p align="center">The Inboxjot command-line interface. <br />Email for builders and their agents.</p>

<p align="center">
  <a href="https://inboxjot.com/docs/cli"><img src="https://inboxjot.com/badges/docs-cli.svg?v=2" alt="Docs" height="24" /></a>
  <a href="./LICENSE"><img src="https://inboxjot.com/badges/license.svg?v=2" alt="License" height="24" /></a>
</p>

Send email, verify domains, and manage contacts, templates, campaigns and webhooks from your terminal, your CI and your agents. One binary, installed as `inboxjot` with the short alias `ij`.

### Quick Start

```sh
npm install -g @inboxjot/cli
# or
brew install inboxjot/tap/inboxjot
```

```sh
inboxjot login
inboxjot domains add acme.com
inboxjot domains verify acme.com --wait
inboxjot send --from hi@acme.com --to delivered@inboxjot.dev --subject "It works" --text "Hello"
```

`inboxjot login` opens the browser and saves the login in the OS keychain. In CI, set `INBOXJOT_API_KEY` instead.

### Commands

| Command | Use |
| --- | --- |
| `login`, `logout`, `whoami` | Log in and see the current workspace |
| `init` | Add a sending key to this project's `.env.local` |
| `send`, `emails` | Send, list, inspect and tail emails |
| `domains` | Add, verify and inspect domains |
| `contacts`, `segments`, `brands`, `topics`, `events` | Manage your audience and track events |
| `templates`, `campaigns`, `automations` | Manage content and sending |
| `webhooks`, `listen`, `trigger` | Manage webhooks and forward live events to localhost |
| `api-keys`, `logs`, `stats` | Keys, request logs and sending stats |
| `api` | Call any API route with your login |
| `doctor` | Check your setup |
| `agent setup` | Add the Inboxjot MCP server and skill to Claude Code, Cursor, Codex and VS Code |

Run `inboxjot help <command>` or see the [CLI docs](https://inboxjot.com/docs/cli) for every option.

### Test Addresses

Email to `delivered@`, `bounced@` or `complained@inboxjot.dev` is never sent. Inboxjot simulates the outcome and sends the usual webhook events.

```sh
inboxjot listen --forward-to localhost:3000/webhooks
inboxjot trigger email.bounced
```

### Scripts and Agents

stdout holds only the answer, and everything else goes to stderr. `--json` prints the API response and `--jq` filters it.

```sh
id=$(inboxjot send --from hi@acme.com --to jane@example.com --subject Hi --text Hello)
inboxjot domains list --jq '.data[].name'
```

When an agent such as Claude Code runs the CLI, output is JSON and prompts are off. Destructive actions then fail with the exact command to run with `--yes`.

### License

[MIT](./LICENSE) &copy; [Arik Chakma](https://x.com/imarikchakma)
