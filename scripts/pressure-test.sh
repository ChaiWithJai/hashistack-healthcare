#!/usr/bin/env bash
# Pressure test: boots (or targets) a control plane and drives the entire
# describe → generate → iterate → gate → deploy → operate → audit workflow
# over real HTTP, asserting every load-bearing behavior — so verification
# never depends on manual smoke testing.
#
# Usage:
#   scripts/pressure-test.sh                # build + boot locally, then test
#   scripts/pressure-test.sh http://host:p  # test an already-running instance
#                                           # (a staging deployment, a container)
#
# TODO(#2): today this exercises the in-process simulation. The staging
# environment ticket points this same script at a control plane backed by a
# Nomad dev agent + Vault dev server, and adds allocation/lease assertions.
set -euo pipefail
cd "$(dirname "$0")/.."

BASE="${1:-}"
SERVER_PID=""

if [[ -z "$BASE" ]]; then
  BASE="http://127.0.0.1:39000"
  cargo build --quiet
  APP_BIND=127.0.0.1:39000 ./target/debug/rust-proof-service &
  SERVER_PID=$!
  trap '[[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true' EXIT
  for _ in $(seq 1 50); do
    curl -sf "$BASE/health" >/dev/null 2>&1 && break
    sleep 0.1
  done
fi

PASS=0; FAIL=0
check() { # check <description> <actual> <expected-substring>
  local desc="$1" actual="$2" want="$3"
  if [[ "$actual" == *"$want"* ]]; then
    PASS=$((PASS+1)); echo "  ok: $desc"
  else
    FAIL=$((FAIL+1)); echo "  FAIL: $desc"; echo "    wanted substring: $want"; echo "    got: ${actual:0:300}"
  fi
}
post() { curl -s -X POST "$BASE$1" -H 'content-type: application/json' -d "${2:-{}}"; }
get()  { curl -s "$BASE$1"; }

echo "== pressure test against $BASE"

echo "-- health & registry"
check "health"            "$(get /health)" '"status":"ok"'
PACKS=$(get /api/packs)
check "5 signed packs"    "$(echo "$PACKS" | python3 -c 'import json,sys; p=json.load(sys.stdin)["packs"]; print(len(p), all(x["signed_by"]=="platform-root-v1" for x in p))')" "5 True"

echo "-- describe → sandbox, synthetic data only"
APP=$(post /api/apps '{"prompt":"a post-op recovery tracker for my knee replacement patients","pack":"post-op-monitor","name":"pt post-op tracker"}')
ID=$(echo "$APP" | python3 -c 'import json,sys; print(json.load(sys.stdin)["app"]["id"])')
check "sandbox stage"     "$APP" '"stage":"sandbox"'
check "synthetic data"    "$APP" '"kind":"synthetic"'

echo "-- gate: 5/6, auto-logoff failing and fixable"
GATE=$(get "/api/apps/$ID/gate")
check "5/6 passed"        "$GATE" '"passed":5'
check "not green"         "$GATE" '"green":false'
check "names auto-logoff" "$GATE" '"id":"auto-logoff"'

echo "-- false-pass guard: promotion locked while failing"
LOCKED=$(post "/api/apps/$ID/promote" '{"cosigner":"Dr. A. Osei"}')
check "409 names check"   "$LOCKED" 'auto-logoff'
STILL=$(get "/api/apps/$ID")
check "still sandboxed"   "$STILL" '"stage":"sandbox"'
check "no allocation"     "$STILL" '"allocation":null'

echo "-- fix it for me, refuse blank co-sign, then promote"
post "/api/apps/$ID/gate/auto-logoff/fix" >/dev/null
NOSIGN=$(post "/api/apps/$ID/promote" '{"cosigner":"  "}')
check "blank co-sign refused" "$NOSIGN" 'co-signature'
LIVE=$(post "/api/apps/$ID/promote" '{"cosigner":"Dr. A. Osei"}')
check "live"              "$LIVE" '"stage":"live"'
check "prod pool"         "$LIVE" '"pool":"prod"'
check "attested 6/6"      "$LIVE" '"gate_summary":"6/6"'

echo "-- nine-gate pack requires platform review"
APP2=$(post /api/apps '{"prompt":"checks each new patient insurance before their first visit","pack":"insurance-verification","name":"pt insurance checker"}')
ID2=$(echo "$APP2" | python3 -c 'import json,sys; print(json.load(sys.stdin)["app"]["id"])')
for g in auto-logoff access-roles escalation-path; do post "/api/apps/$ID2/gate/$g/fix" >/dev/null; done
NOFIX=$(post "/api/apps/$ID2/gate/human-review/fix")
check "review not auto-fixable" "$NOFIX" 'cannot be auto-fixed'
REVIEW=$(post "/api/apps/$ID2/review")
check "reviewer attests"  "$REVIEW" 'Meets release criteria'
LIVE2=$(post "/api/apps/$ID2/promote" '{"cosigner":"Dr. A. Osei"}')
check "9/9 attested"      "$LIVE2" '"gate_summary":"9/9"'

echo "-- eject: an owned bundle, docs from the record, prod-pinned Nomad job"
EXPORT=$(get "/api/apps/$ID/export")
check "job rendered"      "$EXPORT" "job \\\"$ID\\\""
check "prod constraint"   "$EXPORT" 'value     = \"prod\"'
check "no raw tokens"     "$(echo "$EXPORT" | grep -c '{{app_id}}' || true)" "0"
check "compliance doc in bundle" "$EXPORT" '"docs/COMPLIANCE.md"'
check "readme tells their story" "$EXPORT" 'post-op recovery tracker for my knee replacement patients'
check "app becomes a template"   "$EXPORT" "$ID-template"
check "unpack one-liner ships"   "$EXPORT" 'python3 -c'

echo "-- rollback destroys the allocation"
BACK=$(post "/api/apps/$ID/rollback")
check "back to sandbox"   "$BACK" '"stage":"sandbox"'
check "synthetic again"   "$BACK" '"kind":"synthetic"'

echo "-- audit stream: complete story, strictly increasing"
AUDIT=$(get "/api/apps/$ID/audit")
for action in app.created agent.scaffolded gate.fixed gate.passed app.promoted app.rolled_back; do
  check "audit has $action" "$AUDIT" "\"$action\""
done
SEQOK=$(get /api/audit/export | python3 -c '
import json,sys
seqs=[json.loads(l)["seq"] for l in sys.stdin if l.strip()]
print("monotonic" if all(a<b for a,b in zip(seqs,seqs[1:])) and seqs else "broken")')
check "sequence monotonic" "$SEQOK" "monotonic"

echo
echo "== $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
