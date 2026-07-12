#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

compose=(docker compose -f docker-compose.yml)
base="http://127.0.0.1:${STUDIO_PORT:-3000}"
auth=(-H 'authorization: Bearer dev-token-osei')

cleanup() { "${compose[@]}" logs --no-color >.single-host.log 2>&1 || true; }
trap cleanup EXIT

"${compose[@]}" config --quiet
"${compose[@]}" up -d --build --wait
curl -fsS "$base/health" | grep -q '"status":"ok"'

name="portable studio proof $(date +%s)"
app=$(curl -fsS "${auth[@]}" -H 'content-type: application/json' \
  -d "{\"prompt\":\"a post-op recovery tracker for synthetic practice patients\",\"pack\":\"post-op-monitor\",\"name\":\"$name\"}" \
  "$base/api/apps")
id=$(printf '%s' "$app" | python3 -c 'import json,sys; print(json.load(sys.stdin)["app"]["id"])')
curl -fsS "${auth[@]}" -H 'content-type: application/json' -d '{}' \
  "$base/api/apps/$id/gate/auto-logoff/fix" >/dev/null

code=$(curl -sS -o .single-host-real-denial.json -w '%{http_code}' "${auth[@]}" \
  -H 'content-type: application/json' -d '{"cosigner":"Dr. A. Osei"}' \
  "$base/api/apps/$id/promote")
test "$code" = 409
grep -q 'STUBBED' .single-host-real-denial.json

live=$(curl -fsS "${auth[@]}" -H 'content-type: application/json' \
  -d '{"cosigner":"Dr. A. Osei","synthetic_demo":true}' \
  "$base/api/apps/$id/promote")
printf '%s' "$live" | grep -q '"pool":"synthetic-demo"'
printf '%s' "$live" | grep -q '"kind":"synthetic"'
curl -fsS "${auth[@]}" "$base/api/apps/$id/export" | grep -q 'synthetic demo'

"${compose[@]}" restart studio >/dev/null
for _ in $(seq 1 30); do curl -fsS "$base/health" >/dev/null 2>&1 && break; sleep 1; done
curl -fsS "${auth[@]}" "$base/api/apps/$id" | grep -q '"stage":"live"'

echo "single-host proof passed: $id survived restart; real promotion denied; synthetic export succeeded"
