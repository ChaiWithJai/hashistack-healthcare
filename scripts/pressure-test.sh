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
# Staging (#2): with NOMAD_ADDR set (see scripts/staging-up.sh), this same
# script also asserts against real infrastructure — the promoted job is
# registered with the Nomad dev agent and stopped again on rollback; with
# VAULT_ADDR set, the tenant transit key survived a real encrypt/decrypt
# round-trip. Without them those checks print "skipped" and never fail.
#
# Control DB (#7): with CONTROL_DB_URL set, the test additionally kills the
# control plane with SIGKILL mid-flow (right after promote), reboots it
# against the same database, and asserts the app is still live with its
# allocation and audit history intact — the issue's kill -9 bar. The rest
# of the flow then continues against the restarted process, which deepens
# every later assertion.
#
# Audit broker (#8): a self-booted run always attaches a JSONL FileSink
# (AUDIT_FILE), so every load-bearing operation in this test runs under the
# broker invariant — no durable audit write, no operation — and the test
# asserts the HMAC boundary: the doctor's app-scoped view shows their own
# words, while the platform export and the durable archive carry only
# hmac-sha256: forms. Targeting an already-running instance, the AUDIT_FILE
# checks run whenever the env var points at a readable archive.
set -euo pipefail
cd "$(dirname "$0")/.."

BASE="${1:-}"
SERVER_PID=""

if [[ -z "$BASE" ]]; then
  BASE="http://127.0.0.1:39000"
  AUDIT_FILE="${AUDIT_FILE:-$(mktemp -t pressure-audit-XXXXXX.jsonl)}"
  export AUDIT_FILE
  cargo build --quiet
  APP_BIND=127.0.0.1:39000 AUDIT_FILE="$AUDIT_FILE" \
    "${CARGO_TARGET_DIR:-target}/debug/rust-proof-service" &
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

echo "-- gate: auto-logoff failing and fixable; evidence-based verdicts (#3)"
GATE=$(get "/api/apps/$ID/gate")
check "4 passed"          "$GATE" '"passed":4'
check "1 stubbed"         "$GATE" '"stubbed":1'
check "not green"         "$GATE" '"green":false'
check "names auto-logoff" "$GATE" '"id":"auto-logoff"'
# Evidence over claims: verdicts say what they rest on, and the scaffold's
# labeled encryption stub is reported stubbed — never as a pass.
check "evidence basis present"  "$GATE" '"basis":"evidence"'
check "control basis present"   "$GATE" '"basis":"control"'
check "stub never reads as pass" "$(echo "$GATE" | python3 -c 'import json,sys; r=[x for x in json.load(sys.stdin)["report"]["results"] if x["id"]=="phi-encryption"][0]; print(r["basis"], r["status"])')" "evidence stubbed"
# Dual-register vocabulary (P1): HIPAA citations ride the report JSON.
check "citation on audit-log"   "$GATE" '"citation":"45 CFR §164.312(b)"'

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
check "attestation discloses the stub" "$LIVE" '"gate_summary":"5/6 (1 stubbed)"'

echo "-- staging: promote reaches real infrastructure (#2)"
NS="tenant-meridian"
if [[ -n "${NOMAD_ADDR:-}" ]]; then
  check "nomad eval id recorded" "$LIVE" '"nomad_eval_id":"'
  NJOB=$(curl -s "$NOMAD_ADDR/v1/job/$ID?namespace=$NS")
  check "job registered in nomad" "$NJOB" "\"ID\":\"$ID\""
  check "nomad job not stopped"   "$NJOB" '"Stop":false'
else
  echo "  skipped (no nomad): eval id recorded, job registered, job not stopped"
fi
if [[ -n "${VAULT_ADDR:-}" ]]; then
  check "vault transit round-trip" "$LIVE" "\"vault_transit_key\":\"$NS\""
else
  echo "  skipped (no vault): transit round-trip recorded on the allocation"
fi

echo "-- restart survival (#7): kill -9 mid-flow, reboot on the same control DB"
if [[ -n "${CONTROL_DB_URL:-}" ]]; then
  BIND="${BASE#http://}"
  OLD_PID="$SERVER_PID"
  if [[ -z "$OLD_PID" && -f .staging/run/control-plane.pid ]]; then
    OLD_PID=$(cat .staging/run/control-plane.pid)
  fi
  if [[ -z "$OLD_PID" ]]; then
    FAIL=$((FAIL+1)); echo "  FAIL: CONTROL_DB_URL set but no control-plane pid to kill"
  else
    kill -9 "$OLD_PID" 2>/dev/null || true
    for _ in $(seq 1 50); do
      curl -sf "$BASE/health" >/dev/null 2>&1 || break
      sleep 0.1
    done
    cargo build --quiet
    mkdir -p .staging/logs
    env APP_BIND="$BIND" CONTROL_DB_URL="$CONTROL_DB_URL" \
      ${AUDIT_FILE:+AUDIT_FILE="$AUDIT_FILE"} \
      ${NOMAD_ADDR:+NOMAD_ADDR="$NOMAD_ADDR"} \
      ${VAULT_ADDR:+VAULT_ADDR="$VAULT_ADDR"} \
      ${VAULT_TOKEN:+VAULT_TOKEN="$VAULT_TOKEN"} \
      "${CARGO_TARGET_DIR:-target}/debug/rust-proof-service" \
      >>.staging/logs/control-plane.log 2>/dev/null &
    NEW_PID=$!
    # Self-booted server: the existing EXIT trap reads $SERVER_PID at exit,
    # so pointing it at the new pid keeps cleanup working. Staging-managed
    # server: update the pidfile so `staging-up.sh down` still owns it, and
    # leave it running after the test exactly like before.
    if [[ -n "$SERVER_PID" ]]; then
      SERVER_PID=$NEW_PID
    fi
    [[ -f .staging/run/control-plane.pid ]] && echo "$NEW_PID" >.staging/run/control-plane.pid
    for _ in $(seq 1 100); do
      curl -sf "$BASE/health" >/dev/null 2>&1 && break
      sleep 0.1
    done
    SURVIVED=$(get "/api/apps/$ID")
    check "app survives kill -9, still live"   "$SURVIVED" '"stage":"live"'
    check "allocation survives restart"        "$SURVIVED" '"pool":"prod"'
    check "attestation survives restart"       "$SURVIVED" '"gate_summary":"5/6 (1 stubbed)"'
    SAUDIT=$(get "/api/apps/$ID/audit")
    check "audit survives restart (created)"   "$SAUDIT" '"app.created"'
    check "audit survives restart (promoted)"  "$SAUDIT" '"app.promoted"'
    SOPS=$(get "/api/apps/$ID/operations")
    check "operation rows survive restart"     "$SOPS" '"kind":"scaffold"'
    # #8: the promote's audit row is really in postgres, and the prompt is
    # stored in its non-disclosable hmac-sha256: form.
    PSQL=""
    [[ -x .staging/postgres/bin/psql ]] && PSQL=".staging/postgres/bin/psql"
    [[ -z "$PSQL" ]] && command -v psql >/dev/null 2>&1 && PSQL="psql"
    if [[ -n "$PSQL" ]]; then
      NROWS=$($PSQL "$CONTROL_DB_URL" -tAc \
        "SELECT count(*) FROM audit_events WHERE action='app.promoted'" 2>/dev/null || echo err)
      check "postgres holds the promote audit row (#8)" \
        "$([[ "$NROWS" =~ ^[1-9] ]] && echo present || echo "absent:$NROWS")" "present"
      HROWS=$($PSQL "$CONTROL_DB_URL" -tAc \
        "SELECT count(*) FROM audit_events WHERE action='app.created' \
           AND sensitive->>'prompt' LIKE 'hmac-sha256:%'" 2>/dev/null || echo err)
      check "postgres stores the prompt hmac'd (#8)" \
        "$([[ "$HROWS" =~ ^[1-9] ]] && echo present || echo "absent:$HROWS")" "present"
    else
      echo "  skipped (no psql): promote audit row + hmac'd prompt in postgres"
    fi
  fi
else
  echo "  skipped (no CONTROL_DB_URL): app, allocation, audit, operations survive restart"
fi

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
# post-op-monitor is converted to the runnable-scaffold spec (#5): the
# bundle carries real app source and the runbook drops its old caveat.
check "real scaffold source ships" "$EXPORT" '"app/src/main.rs"'
check "runbook drops placeholder caveat" "$(echo "$EXPORT" | python3 -c 'import json,sys; rb=json.load(sys.stdin)["files"]["docs/RUNBOOK.md"]; print("caveat-present" if "scaffold placeholder" in rb else "real-source")')" "real-source"
# F1 (review-log round 1): staging submission strips the vault stanza for
# the dev agent, but the RENDERED job text the doctor owns must always
# carry it — the stripped path may never quietly become load-bearing.
check "rendered job keeps vault stanza" "$(echo "$EXPORT" | python3 -c 'import json,sys; job=json.load(sys.stdin)["files"]["nomad/job.nomad.hcl"]; print("stanza-present" if "vault {" in job else "stanza-missing")')" "stanza-present"
# F3: a released app's compliance record embeds the report frozen at
# promotion — the evidence that admitted it — never a lineage re-run.
COMPLIANCE=$(echo "$EXPORT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["files"]["docs/COMPLIANCE.md"])')
check "compliance report frozen at promotion" "$COMPLIANCE" 'frozen at promotion'
check "compliance names the stub"             "$COMPLIANCE" 'STUBBED —'
check "compliance carries HIPAA citations"    "$COMPLIANCE" '45 CFR §164.312(b)'
# The adversarial broken scaffold (#3) is cargo-test-only fixture data:
# it must never register as a pack or reach any API surface.
check "adversarial fixture is not a shipped pack" "$(get /api/packs | python3 -c 'import json,sys; p=json.load(sys.stdin)["packs"]; print("absent" if not any("broken" in x["id"] for x in p) and len(p)==5 else "present")')" "absent"

echo "-- rollback destroys the allocation"
BACK=$(post "/api/apps/$ID/rollback")
check "back to sandbox"   "$BACK" '"stage":"sandbox"'
check "synthetic again"   "$BACK" '"kind":"synthetic"'
if [[ -n "${NOMAD_ADDR:-}" ]]; then
  NSTOP=$(curl -s "$NOMAD_ADDR/v1/job/$ID?namespace=$NS")
  check "nomad job stopped on rollback" "$NSTOP" '"Stop":true'
else
  echo "  skipped (no nomad): nomad job stopped on rollback"
fi

echo "-- routing ladder (#4, decision 0001): verified ops, no model env needed"
ITER=$(post "/api/apps/$ID/iterate" '{"instruction":"remind patients to log their wound photos daily"}')
check "iterate lands"     "$ITER" '"added_feature"'
OPS=$(get "/api/apps/$ID/operations")
check "operation recorded"        "$OPS" '"kind":"iterate"'
check "operation settled success" "$OPS" '"status":"success"'
RAUDIT=$(get "/api/apps/$ID/audit")
check "audit has agent.attempt"      "$RAUDIT" '"agent.attempt"'
check "rules tier verdict accepted"  "$RAUDIT" 'tier=rules verdict=accepted'
check "pack routing policy cited"    "$(get "/api/apps/$ID2/audit")" 'per pack insurance-verification routing policy'

echo "-- audit stream: complete story, strictly increasing"
AUDIT=$(get "/api/apps/$ID/audit")
for action in app.created agent.scaffolded gate.fixed gate.passed app.promoted app.rolled_back; do
  check "audit has $action" "$AUDIT" "\"$action\""
done
if [[ -n "${NOMAD_ADDR:-}" ]]; then
  check "audit has nomad.job_submitted" "$AUDIT" '"nomad.job_submitted"'
  check "audit has nomad.job_stopped"   "$AUDIT" '"nomad.job_stopped"'
else
  echo "  skipped (no nomad): audit has nomad.job_submitted, nomad.job_stopped"
fi
if [[ -n "${VAULT_ADDR:-}" ]]; then
  check "audit has vault.transit_verified" "$AUDIT" '"vault.transit_verified"'
else
  echo "  skipped (no vault): audit has vault.transit_verified"
fi
SEQOK=$(get /api/audit/export | python3 -c '
import json,sys
seqs=[json.loads(l)["seq"] for l in sys.stdin if l.strip()]
print("monotonic" if all(a<b for a,b in zip(seqs,seqs[1:])) and seqs else "broken")')
check "sequence monotonic" "$SEQOK" "monotonic"

echo "-- audit broker (#8): HMAC boundary + durable JSONL archive"
WORDS="knee replacement patients"
APPAUD=$(get "/api/apps/$ID/audit")
check "app-scoped view shows the doctor's words" "$APPAUD" "$WORDS"
EXPORT_STREAM=$(get /api/audit/export)
check "platform export carries the hmac form"    "$EXPORT_STREAM" 'hmac-sha256:'
check "platform export hides the words" \
  "$(echo "$EXPORT_STREAM" | grep -c "$WORDS" || true)" "0"
COMPLIANCE8=$(get "/api/apps/$ID/export" | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["files"]["docs/COMPLIANCE.md"])')
check "ejected compliance keeps the doctor's words" "$COMPLIANCE8" "$WORDS"
if [[ -n "${AUDIT_FILE:-}" && -r "$AUDIT_FILE" ]]; then
  check "archive has the registration probe line" "$(cat "$AUDIT_FILE")" '"audit.sink_probe"'
  check "archive holds app.promoted durably"      "$(cat "$AUDIT_FILE")" '"app.promoted"'
  check "archive carries the hmac form"           "$(cat "$AUDIT_FILE")" 'hmac-sha256:'
  check "archive hides the words" "$(grep -c "$WORDS" "$AUDIT_FILE" || true)" "0"
else
  echo "  skipped (no AUDIT_FILE): probe line, app.promoted, hmac form, no plaintext in archive"
fi

echo
echo "== $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
