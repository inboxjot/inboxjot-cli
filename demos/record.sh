#!/usr/bin/env bash
# Records the demos in demos/out with VHS against a local Inboxjot API.
# Needs a config folder that is logged in to a workspace with a verified acme.dev domain.
# Usage: INBOXJOT_DEMO_CONFIG=/path/to/config demos/record.sh [send webhooks contacts init agent]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$root/target/demos"
config="${INBOXJOT_DEMO_CONFIG:?Set INBOXJOT_DEMO_CONFIG to a logged-in config folder}"
tapes=("$@")
if [ ${#tapes[@]} -eq 0 ]; then
  tapes=(send webhooks contacts init agent)
fi

cd "$root"
cargo build --release --quiet

rm -rf "$work"
mkdir -p "$work/bin" "$work/agent-bin" "$work/agent-home/.claude" \
  "$work/acme-app" "$root/demos/out"
ln -s "$root/target/release/inboxjot" "$work/bin/inboxjot"
ln -s "$root/target/release/inboxjot" "$work/bin/ij"
ln -s "$root/target/release/inboxjot" "$work/agent-bin/inboxjot"
printf '#!/bin/sh\nexit 0\n' > "$work/agent-bin/claude"
chmod +x "$work/agent-bin/claude"
cp "$root/demos/customers.csv" "$work/acme-app/"
printf '{ "name": "acme-app", "private": true }\n' > "$work/acme-app/package.json"
printf 'node_modules\n' > "$work/acme-app/.gitignore"

cat > "$work/rc.sh" <<'EOF'
PS1='\[\e[38;5;241m\]acme-app\[\e[0m\] ❯ '
PS2='  '
set +m
cd "$INBOXJOT_DEMO_PROJECT"
EOF

unset CLAUDECODE CODEX_SANDBOX GEMINI_CLI AGENT CI
export PATH="$work/bin:$PATH"
export INBOXJOT_API_URL="${INBOXJOT_API_URL:-http://localhost:8900}"
export INBOXJOT_DASHBOARD_URL="${INBOXJOT_DASHBOARD_URL:-http://localhost:3900}"
export INBOXJOT_CONFIG_DIR="$config"
export __INBOXJOT_FILE_CREDENTIALS=1
export INBOXJOT_DEMO_RC="$work/rc.sh"
export INBOXJOT_DEMO_PROJECT="$work/acme-app"
export INBOXJOT_DEMO_AGENT_HOME="$work/agent-home"
export INBOXJOT_DEMO_AGENT_PATH="$work/agent-bin:/usr/bin:/bin"

python3 -I -c '
from http.server import BaseHTTPRequestHandler, HTTPServer

class Receiver(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.send_response(200)
        self.end_headers()

    def log_message(self, *args):
        pass

HTTPServer(("127.0.0.1", 4242), Receiver).serve_forever()
' &
receiver=$!
trap 'kill "$receiver" 2>/dev/null' EXIT

for tape in "${tapes[@]}"; do
  if [ "$tape" = contacts ]; then
    for email in $(tail -n +2 demos/customers.csv | cut -d, -f1); do
      inboxjot contacts delete "$email" --yes >/dev/null 2>&1 || true
    done
  fi
  echo "Recording $tape"
  vhs "demos/$tape.tape"
done
