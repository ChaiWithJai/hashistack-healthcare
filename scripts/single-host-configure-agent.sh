#!/usr/bin/env bash
# Atomically install the bounded agent routing values on one studio host.
# Secrets travel over SSH stdin and are never command-line arguments.
set -euo pipefail

host="${1:?usage: scripts/single-host-configure-agent.sh HOST}"
: "${DIGITALOCEAN_PLANNER_ENDPOINT:?DIGITALOCEAN_PLANNER_ENDPOINT is required}"
: "${DIGITALOCEAN_PLANNER_ACCESS_KEY:?DIGITALOCEAN_PLANNER_ACCESS_KEY is required}"
: "${DIGITALOCEAN_PLANNER_VERSION:?DIGITALOCEAN_PLANNER_VERSION is required}"
: "${WORKSPACE_AGENT_TIMEOUT_SECS:=20}"
readonly MAX_HOSTED_AGENT_TIMEOUT_SECS=20

case "$WORKSPACE_AGENT_TIMEOUT_SECS" in
  ''|*[!0-9]*) echo "WORKSPACE_AGENT_TIMEOUT_SECS must be an integer from 1 to 120" >&2; exit 1 ;;
esac
if [ "$WORKSPACE_AGENT_TIMEOUT_SECS" -lt 1 ] || [ "$WORKSPACE_AGENT_TIMEOUT_SECS" -gt 120 ]; then
  echo "WORKSPACE_AGENT_TIMEOUT_SECS must be an integer from 1 to 120" >&2
  exit 1
fi
if [ "$WORKSPACE_AGENT_TIMEOUT_SECS" -gt "$MAX_HOSTED_AGENT_TIMEOUT_SECS" ]; then
  echo "capping WORKSPACE_AGENT_TIMEOUT_SECS=$WORKSPACE_AGENT_TIMEOUT_SECS to $MAX_HOSTED_AGENT_TIMEOUT_SECS so the preview proxy can receive the fallback" >&2
  WORKSPACE_AGENT_TIMEOUT_SECS="$MAX_HOSTED_AGENT_TIMEOUT_SECS"
fi

values=(
  "WORKSPACE_AGENT_PROVIDER=digitalocean"
  "DIGITALOCEAN_PLANNER_ENDPOINT=$DIGITALOCEAN_PLANNER_ENDPOINT"
  "DIGITALOCEAN_PLANNER_ACCESS_KEY=$DIGITALOCEAN_PLANNER_ACCESS_KEY"
  "DIGITALOCEAN_PLANNER_VERSION=$DIGITALOCEAN_PLANNER_VERSION"
  "WORKSPACE_AGENT_TIMEOUT_SECS=$WORKSPACE_AGENT_TIMEOUT_SECS"
)

for value in "${values[@]}"; do
  if [[ "$value" == *$'\n'* || "$value" == *$'\r'* ]]; then
    echo "agent environment values must be one line" >&2
    exit 1
  fi
done

printf '%s\n' "${values[@]}" | ssh -o BatchMode=yes "$host" 'set -eu
  file=/etc/hashistack-studio.env
  test -f "$file"
  tmp=$(mktemp)
  trap '\''rm -f "$tmp"'\'' EXIT
  cp "$file" "$tmp"
  while IFS= read -r line; do
    key=${line%%=*}
    case "$key" in
      WORKSPACE_AGENT_PROVIDER|DIGITALOCEAN_PLANNER_ENDPOINT|DIGITALOCEAN_PLANNER_ACCESS_KEY|DIGITALOCEAN_PLANNER_VERSION|WORKSPACE_AGENT_TIMEOUT_SECS) ;;
      *) echo "refusing unknown agent environment key" >&2; exit 1 ;;
    esac
    sed -i "/^${key}=/d" "$tmp"
    printf "%s\n" "$line" >> "$tmp"
  done
  install -o root -g root -m 0600 "$tmp" "$file"'

echo "installed versioned DigitalOcean agent routing on $host"
