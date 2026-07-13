#!/usr/bin/env bash
# Provider-neutral post-provision proof for a DO droplet or GCP VM.
set -euo pipefail

base="${1:?usage: scripts/single-host-remote-proof.sh http://HOST:3000}"
base="${base%/}"
json=(-H 'content-type: application/json')
name="remote portability proof $(date +%s)"

curl -fsS "$base/health" | grep -q '"status":"ok"'
auth_mode=$(curl -fsS "$base/auth/config" | python3 -c 'import json,sys; print(json.load(sys.stdin)["mode"])')
code=$(curl -sS -o /tmp/hashistack-unauthorized.json -w '%{http_code}' \
  "$base/api/apps")
test "$code" = 401

if [ "$auth_mode" = clerk ]; then
  token="${STUDIO_CLERK_TOKEN:-}"
  cookie=$(mktemp)
  trap 'rm -f "$cookie"' EXIT
  curl -fsS -c "$cookie" -H "origin: $base" "${json[@]}" -d '{}' "$base/api/public/session" >/dev/null
  auth=(-b "$cookie" -H "origin: $base")
else
  token="${STUDIO_BEARER_TOKEN:-dev-token-osei}"
  auth=(-H "authorization: Bearer $token")
fi

app=$(curl -fsS "${auth[@]}" "${json[@]}" \
  -d "{\"prompt\":\"a post-op recovery tracker for synthetic practice patients\",\"pack\":\"post-op-monitor\",\"name\":\"$name\"}" \
  "$base/api/apps")
id=$(printf '%s' "$app" | python3 -c 'import json,sys; print(json.load(sys.stdin)["app"]["id"])')
curl -fsS "${auth[@]}" "${json[@]}" -d '{}' \
  "$base/api/apps/$id/gate/auto-logoff/fix" >/dev/null

if [ "$auth_mode" = clerk ]; then
  code=$(curl -sS -o /tmp/hashistack-real-denial.json -w '%{http_code}' \
    "${auth[@]}" "${json[@]}" -d '{}' "$base/api/apps/$id/promote")
  test "$code" = 403
  grep -q 'sign in' /tmp/hashistack-real-denial.json
  release='{"synthetic_demo":true}'
else
  code=$(curl -sS -o /tmp/hashistack-real-denial.json -w '%{http_code}' \
    "${auth[@]}" "${json[@]}" -d '{"cosigner":"Dr. A. Osei"}' \
    "$base/api/apps/$id/promote")
  test "$code" = 409
  grep -q 'STUBBED' /tmp/hashistack-real-denial.json
  release='{"cosigner":"Dr. A. Osei","synthetic_demo":true}'
fi

live=$(curl -fsS "${auth[@]}" "${json[@]}" \
  -d "$release" \
  "$base/api/apps/$id/promote")
printf '%s' "$live" | grep -q '"pool":"synthetic-demo"'
printf '%s' "$live" | grep -q '"kind":"synthetic"'

operate=$(curl -fsS "${auth[@]}" "$base/api/apps/$id/operate")
printf '%s' "$operate" | grep -q '"available":false'
printf '%s' "$operate" | grep -q '"healthy":false'

if [ "$auth_mode" = clerk ]; then
  code=$(curl -sS -o /tmp/hashistack-guest-export.json -w '%{http_code}' \
    "${auth[@]}" "$base/api/apps/$id/export")
  test "$code" = 401
  if [ -z "$token" ]; then
    echo "remote public flow passed at $base: $id was built, repaired, and published to the synthetic pool; export correctly requires Clerk"
    exit 0
  fi
  owner=(-H "authorization: Bearer $token" -b "$cookie")
  curl -fsS "${owner[@]}" "${json[@]}" -d '{}' "$base/api/apps/$id/claim" >/dev/null
  bundle=$(curl -fsS "${owner[@]}" "$base/api/apps/$id/export")
else
  bundle=$(curl -fsS "${auth[@]}" "$base/api/apps/$id/export")
fi
printf '%s' "$bundle" | grep -q 'synthetic demo'
printf '%s' "$bundle" | grep -q 'server/src/main.rs'
printf '%s' "$bundle" | grep -q 'web/src/routes/+page.svelte'

echo "remote proof passed at $base: $id is synthetic-only, exportable, and makes no fabricated telemetry claim"
