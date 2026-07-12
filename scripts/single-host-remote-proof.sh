#!/usr/bin/env bash
# Provider-neutral post-provision proof for a DO droplet or GCP VM.
set -euo pipefail

base="${1:?usage: scripts/single-host-remote-proof.sh http://HOST:3000}"
base="${base%/}"
token="${STUDIO_BEARER_TOKEN:-dev-token-osei}"
auth=(-H "authorization: Bearer $token")
json=(-H 'content-type: application/json')
name="remote portability proof $(date +%s)"

curl -fsS "$base/health" | grep -q '"status":"ok"'
code=$(curl -sS -o /tmp/hashistack-unauthorized.json -w '%{http_code}' \
  "$base/api/apps")
test "$code" = 401

app=$(curl -fsS "${auth[@]}" "${json[@]}" \
  -d "{\"prompt\":\"a post-op recovery tracker for synthetic practice patients\",\"pack\":\"post-op-monitor\",\"name\":\"$name\"}" \
  "$base/api/apps")
id=$(printf '%s' "$app" | python3 -c 'import json,sys; print(json.load(sys.stdin)["app"]["id"])')
curl -fsS "${auth[@]}" "${json[@]}" -d '{}' \
  "$base/api/apps/$id/gate/auto-logoff/fix" >/dev/null

code=$(curl -sS -o /tmp/hashistack-real-denial.json -w '%{http_code}' \
  "${auth[@]}" "${json[@]}" -d '{"cosigner":"Dr. A. Osei"}' \
  "$base/api/apps/$id/promote")
test "$code" = 409
grep -q 'STUBBED' /tmp/hashistack-real-denial.json

live=$(curl -fsS "${auth[@]}" "${json[@]}" \
  -d '{"cosigner":"Dr. A. Osei","synthetic_demo":true}' \
  "$base/api/apps/$id/promote")
printf '%s' "$live" | grep -q '"pool":"synthetic-demo"'
printf '%s' "$live" | grep -q '"kind":"synthetic"'

operate=$(curl -fsS "${auth[@]}" "$base/api/apps/$id/operate")
printf '%s' "$operate" | grep -q '"available":false'
printf '%s' "$operate" | grep -q '"healthy":false'

bundle=$(curl -fsS "${auth[@]}" "$base/api/apps/$id/export")
printf '%s' "$bundle" | grep -q 'synthetic demo'
printf '%s' "$bundle" | grep -q 'app/src/main.rs'

echo "remote proof passed at $base: $id is synthetic-only, exportable, and makes no fabricated telemetry claim"
