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
#
# Vault dynamic creds (#9): with VAULT_ADDR + CONTROL_DB_URL both set, the
# test asserts the compliance spine live — the promoted allocation carries a
# real database-engine lease (verified by the control plane with SELECT 1 as
# the issued user), a sibling lease authenticates and then FAILS after
# revocation (password held by this test), the per-tenant ACL policy reads
# back from sys/policies/acl, the tenant transit key rotates with old
# ciphertext still decryptable, Vault's file audit device carries the
# transit request path, and no password ever appears in the platform audit
# export. Without VAULT_ADDR those checks print "skipped" and never fail.
#
# Identity (#10): every request rides `Authorization: Bearer dev-token-osei`
# (the dev registry's meridian clinician), so this same script also passes
# against a strict staging instance (IDENTITIES_FILE set — export it for the
# run so the kill -9 reboot keeps it and the dev-fallback checks skip). The
# identity section drives a second tenant (dr-park) and a staff principal
# with explicit tokens against the SAME instance — cross-tenant 404s, role
# 403s, the attestation digest — and, on self-booted runs, additionally
# proves the dev fallback (headerless request works + is confessed in the
# audit stream) and a strict instance's 401s + 1s session idle expiry.
set -euo pipefail
cd "$(dirname "$0")/.."

BASE="${1:-}"
SERVER_PID=""
STRICT_PID=""

if [[ -z "$BASE" ]]; then
  BASE="http://127.0.0.1:39000"
  AUDIT_FILE="${AUDIT_FILE:-$(mktemp -t pressure-audit-XXXXXX.jsonl)}"
  export AUDIT_FILE
  cargo build --quiet
  APP_BIND=127.0.0.1:39000 AUDIT_FILE="$AUDIT_FILE" \
    "${CARGO_TARGET_DIR:-target}/debug/rust-proof-service" &
  SERVER_PID=$!
  trap '{ [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null; [[ -n "$STRICT_PID" ]] && kill "$STRICT_PID" 2>/dev/null; } || true' EXIT
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
# #10: the doctor identity every main-flow request authenticates as — the
# same principal the dev fallback resolves, so assertions are mode-invariant.
TOKEN="dev-token-osei"
post() { curl -s -X POST "$BASE$1" -H 'content-type: application/json' \
  -H "authorization: Bearer ${3:-$TOKEN}" -d "${2:-{}}"; }
get()  { curl -s -H "authorization: Bearer ${2:-$TOKEN}" "$BASE$1"; }
code() { # code <method> <path> <token-or-"-"> [json-body] → HTTP status only
  local auth=(); [[ "$3" != "-" ]] && auth=(-H "authorization: Bearer $3")
  curl -s -o /dev/null -w '%{http_code}' -X "$1" "${auth[@]}" \
    ${4:+-H content-type:application/json -d "$4"} "$BASE$2"
}
vault_api() { # vault_api <method> <path> [json-body]
  curl -s -H "x-vault-token: ${VAULT_TOKEN:-staging-root}" -X "$1" \
    ${3:+-d "$3"} "$VAULT_ADDR/v1$2"
}
jfield() { python3 -c "import json,sys; print(json.load(sys.stdin)$1)"; }

# psql, wherever it lives (staging's portable postgres, or the system one) —
# used by the #7 restart evidence and the #9 credential-lifecycle evidence.
PSQL=""
[[ -x .staging/postgres/bin/psql ]] && PSQL=".staging/postgres/bin/psql"
[[ -z "$PSQL" ]] && command -v psql >/dev/null 2>&1 && PSQL="psql"

echo "== pressure test against $BASE"

echo "-- health & registry"
check "health"            "$(get /health)" '"status":"ok"'
PACKS=$(get /api/packs)
check "17 signed packs"   "$(echo "$PACKS" | python3 -c 'import json,sys; p=json.load(sys.stdin)["packs"]; print(len(p), all(x["signed_by"]=="platform-root-v1" for x in p))')" "17 True"

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
NOSIGN=$(post "/api/apps/$ID/promote" '{"cosigner":"  ","synthetic_demo":true}')
check "blank co-sign refused" "$NOSIGN" 'co-signature'
# #10: the typed field is only a display-name check against the
# authenticated principal — naming anyone else is refused.
WRONGSIGN=$(post "/api/apps/$ID/promote" '{"cosigner":"Dr. Somebody Else","synthetic_demo":true}')
check "mismatched co-sign name refused" "$WRONGSIGN" 'co-signature'
LIVE=$(post "/api/apps/$ID/promote" '{"cosigner":"Dr. A. Osei","synthetic_demo":true}')
check "live"              "$LIVE" '"stage":"live"'
check "synthetic demo pool" "$LIVE" '"pool":"synthetic-demo"'
check "stub demo never receives tenant data" "$LIVE" '"data_source":{"kind":"synthetic"'
check "attestation discloses the stub" "$LIVE" '"gate_summary":"5/6 (1 stubbed)"'
# #10: the co-sign is cryptographic — the attestation binds the
# authenticated principal and a sha256 digest of the frozen gate report.
check "attestation binds the principal"     "$LIVE" '"principal":"dr-osei"'
check "attestation carries the report digest" "$LIVE" '"report_digest":"sha256:'

# The post-op reference intentionally contains a labeled encryption stub, so
# it may only enter the synthetic-demo pool. Infrastructure proof uses a
# separate no-stub app; mixing these two paths used to make the staging test
# expect a production Nomad job and Vault lease for a synthetic demo.
PROOF_ID="$ID"
PROOF_LIVE="$LIVE"
if [[ -n "${NOMAD_ADDR:-}${VAULT_ADDR:-}${CONTROL_DB_URL:-}" ]]; then
  INFRA=$(post /api/apps '{"prompt":"track home blood pressure and route urgent readings for review","pack":"hypertension-tracker","name":"infrastructure proof"}')
  PROOF_ID=$(echo "$INFRA" | jfield '["app"]["id"]')
  for g in auto-logoff escalation-path; do
    post "/api/apps/$PROOF_ID/gate/$g/fix" >/dev/null
  done
  PROOF_LIVE=$(post "/api/apps/$PROOF_ID/promote" '{"cosigner":"Dr. A. Osei"}')
  check "infrastructure proof has no stubs" "$PROOF_LIVE" '"gate_summary":"7/7"'
  check "infrastructure proof enters prod pool" "$PROOF_LIVE" '"pool":"prod"'
fi

echo "-- staging: promote reaches real infrastructure (#2)"
NS="tenant-meridian"
# #6 (honest slice): operate reports Nomad's dual status axes. Desired is
# the record's claim; observed is polled from the REAL job in staging
# (status_source nomad — an honest "pending" on the one-machine dev agent,
# where role=prod is unsatisfiable) and mirrors desired in simulated mode
# (status_source simulated — labeled, never claimed).
OPERATE=$(get "/api/apps/$PROOF_ID/operate")
check "operate: desired axis is the record" "$OPERATE" '"desired_state":"running"'
check "operate: observed axis present"      "$OPERATE" '"observed_state":"'
if [[ -n "${NOMAD_ADDR:-}" ]]; then
  check "nomad eval id recorded" "$PROOF_LIVE" '"nomad_eval_id":"'
  NJOB=$(curl -s "$NOMAD_ADDR/v1/job/$PROOF_ID?namespace=$NS")
  check "job registered in nomad" "$NJOB" "\"ID\":\"$PROOF_ID\""
  check "nomad job not stopped"   "$NJOB" '"Stop":false'
  # The observation must really come from Nomad, and must agree with what
  # Nomad itself says about the job right now.
  check "operate: observed from real nomad" "$OPERATE" '"status_source":"nomad"'
  NSTATUS=$(echo "$NJOB" | jfield '["Status"]')
  check "operate: observed matches nomad's own word" "$OPERATE" "\"observed_state\":\"$NSTATUS\""
  if [[ "${NOMAD_REQUIRE_ALLOCATION:-}" == "1" ]]; then
    ALLOCS="[]"
    for _ in $(seq 1 60); do
      ALLOCS=$(curl -s "$NOMAD_ADDR/v1/job/$PROOF_ID/allocations?namespace=$NS")
      ASTATUS=$(echo "$ALLOCS" | python3 -c \
        'import json,sys; a=json.load(sys.stdin); print(a[0]["ClientStatus"] if a else "missing")')
      [[ "$ASTATUS" == "running" || "$ASTATUS" == "failed" ]] && break
      sleep 0.5
    done
    check "nomad allocation reaches running" "$ASTATUS" "running"
    ALLOC_ID=$(echo "$ALLOCS" | jfield '[0]["ID"]')
    ADETAIL=$(curl -s "$NOMAD_ADDR/v1/allocation/$ALLOC_ID?namespace=$NS")
    APORT=$(echo "$ADETAIL" | jfield '["AllocatedResources"]["Shared"]["Ports"][0]["Value"]')
    AHEALTH=$(curl -s "http://127.0.0.1:$APORT/health")
    check "nomad allocation health route answers" "$AHEALTH" '"status":"ok"'
    check "nomad allocation keeps synthetic dataset guard" "$AHEALTH" '"synthetic_only":true'
  fi
else
  echo "  skipped (no nomad): eval id recorded, job registered, job not stopped"
  check "operate: simulated mode reports desired=observed, labeled" "$OPERATE" '"status_source":"simulated"'
  check "operate: simulated observed mirrors desired" "$OPERATE" '"observed_state":"running"'
fi
if [[ -n "${VAULT_ADDR:-}" ]]; then
  check "vault transit round-trip" "$PROOF_LIVE" "\"vault_transit_key\":\"$NS\""
else
  echo "  skipped (no vault): transit round-trip recorded on the allocation"
fi

echo "-- vault (#9): per-tenant policy, key rotation, audit device"
if [[ -n "${VAULT_ADDR:-}" ]]; then
  # The policy mounted at first promote, read back from Vault itself.
  POL=$(vault_api GET "/sys/policies/acl/$NS")
  check "tenant policy mounted (sys/policies/acl read-back)" "$POL" "transit/encrypt/$NS"
  check "tenant policy names the decrypt path"               "$POL" "transit/decrypt/$NS"
  check "tenant policy names the database path"              "$POL" 'database/creds/tenant-app'

  # Rotate-proof: encrypt, rotate the tenant key, decrypt the PRE-rotation
  # ciphertext — key versioning means rotation never strands old data.
  RPROBE=$(printf 'rotate-proof-%s' "$PROOF_ID" | base64)
  CT=$(vault_api POST "/transit/encrypt/$NS" "{\"plaintext\":\"$RPROBE\"}" \
    | jfield '["data"]["ciphertext"]')
  vault_api POST "/transit/keys/$NS/rotate" >/dev/null
  KVER=$(vault_api GET "/transit/keys/$NS" | jfield '["data"]["latest_version"]')
  check "tenant key rotated (version advanced)" \
    "$([[ "$KVER" -ge 2 ]] && echo advanced || echo "still v$KVER")" "advanced"
  PT=$(vault_api POST "/transit/decrypt/$NS" "{\"ciphertext\":\"$CT\"}" \
    | jfield '["data"]["plaintext"]')
  check "pre-rotation ciphertext decrypts after rotate" "$PT" "$RPROBE"

  # Vault's own file audit device (staging-up.sh enables it at boot): the
  # HIPAA technical-safeguard artifact, carrying the transit request path.
  VAF=".staging/logs/vault-audit.log"
  if [[ -r "$VAF" ]]; then
    check "vault audit file non-empty" \
      "$([[ -s "$VAF" ]] && echo non-empty || echo empty)" "non-empty"
    check "vault audit logs the transit request path" \
      "$(grep -q "transit/encrypt/$NS" "$VAF" && echo present || echo absent)" "present"
  else
    echo "  skipped (no local vault audit file): non-empty, transit path logged"
  fi
else
  echo "  skipped (no vault): policy read-back, rotate-proof, vault audit file"
fi

echo "-- vault (#9): dynamic db creds issued, verified, and revocable"
PLAT_VUSER=""
PLAT_LEASE=""
SIB_PASS=""
if [[ -n "${VAULT_ADDR:-}" && -n "${CONTROL_DB_URL:-}" && -n "$PSQL" ]]; then
  check "allocation carries the lease id"    "$PROOF_LIVE" '"vault_lease_id":"database/creds/tenant-app/'
  check "allocation carries the issued user" "$PROOF_LIVE" '"vault_db_username":"v-'
  check "allocation ttl is 1h"               "$PROOF_LIVE" '"vault_lease_ttl_secs":3600'
  check "credentials string is the real lease, not the placeholder" \
    "$PROOF_LIVE" 'vault database/creds/tenant-app: lease'
  PLAT_VUSER=$(echo "$PROOF_LIVE" \
    | jfield '["app"]["allocation"].get("vault_db_username") or ""')
  PLAT_LEASE=$(echo "$PROOF_LIVE" \
    | jfield '["app"]["allocation"].get("vault_lease_id") or ""')

  # A sibling lease from the same role, password in hand — the literal
  # authenticate-then-revoke-then-fail evidence (the platform never
  # discloses its own lease's password, by design).
  SIB=$(vault_api GET "/database/creds/tenant-app")
  SIB_USER=$(echo "$SIB" | jfield '["data"]["username"]')
  SIB_PASS=$(echo "$SIB" | jfield '["data"]["password"]')
  SIB_LEASE=$(echo "$SIB" | jfield '["lease_id"]')
  SIB_URL=$(echo "$CONTROL_DB_URL" | sed -E "s#//[^@]+@#//$SIB_USER:$SIB_PASS@#")
  check "issued creds authenticate (SELECT 1 as the issued user)" \
    "$($PSQL "$SIB_URL" -tAc 'SELECT 1' 2>/dev/null | grep -qx 1 && echo authenticated || echo refused)" \
    "authenticated"
  vault_api PUT "/sys/leases/revoke" "{\"lease_id\":\"$SIB_LEASE\"}" >/dev/null
  check "revoked creds fail to authenticate" \
    "$($PSQL "$SIB_URL" -tAc 'SELECT 1' 2>/dev/null | grep -qx 1 && echo authenticated || echo refused)" \
    "refused"
  check "revoked role dropped from pg_roles" \
    "$($PSQL "$CONTROL_DB_URL" -tAc "SELECT count(*) FROM pg_roles WHERE rolname='$SIB_USER'")" "0"
else
  echo "  skipped (no vault+control-db+psql): lease on the allocation, creds"
  echo "  authenticate as the issued user, revoked creds fail, role dropped"
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
      ${NOMAD_STAGING_IMAGE:+NOMAD_STAGING_IMAGE="$NOMAD_STAGING_IMAGE"} \
      ${VAULT_ADDR:+VAULT_ADDR="$VAULT_ADDR"} \
      ${VAULT_TOKEN:+VAULT_TOKEN="$VAULT_TOKEN"} \
      ${IDENTITIES_FILE:+IDENTITIES_FILE="$IDENTITIES_FILE"} \
      ${SESSION_IDLE_SECS:+SESSION_IDLE_SECS="$SESSION_IDLE_SECS"} \
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
    SURVIVED=$(get "/api/apps/$PROOF_ID")
    check "app survives kill -9, still live"   "$SURVIVED" '"stage":"live"'
    check "allocation survives restart"        "$SURVIVED" '"pool":"prod"'
    check "attestation survives restart"       "$SURVIVED" '"gate_summary":"7/7"'
    SAUDIT=$(get "/api/apps/$PROOF_ID/audit")
    check "audit survives restart (created)"   "$SAUDIT" '"app.created"'
    check "audit survives restart (promoted)"  "$SAUDIT" '"app.promoted"'
    SOPS=$(get "/api/apps/$PROOF_ID/operations")
    check "operation rows survive restart"     "$SOPS" '"kind":"scaffold"'
    # #9: the lease HANDLE survives the restart (the password does not — the
    # control plane persists no secrets), so rollback can still revoke it.
    if [[ -n "${VAULT_ADDR:-}" ]]; then
      check "vault lease handle survives restart" "$SURVIVED" '"vault_lease_id":"database/creds/tenant-app/'
    fi
    # #8: the promote's audit row is really in postgres, and the prompt is
    # stored in its non-disclosable hmac-sha256: form.
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

echo "-- eject: an owned bundle, docs from the record, placement-safe Nomad job"
EXPORT=$(get "/api/apps/$ID/export")
check "job rendered"      "$EXPORT" "job \\\"$ID\\\""
check "synthetic-demo constraint" "$EXPORT" 'value     = \"synthetic-demo\"'
check "no raw tokens"     "$(echo "$EXPORT" | grep -c '{{app_id}}' || true)" "0"
check "compliance doc in bundle" "$EXPORT" '"docs/COMPLIANCE.md"'
check "readme tells their story" "$EXPORT" 'post-op recovery tracker for my knee replacement patients'
check "app becomes a template"   "$EXPORT" "$ID-template"
check "unpack one-liner ships"   "$EXPORT" 'python3 -c'
# post-op-monitor is converted to the runnable-scaffold spec (#5): the
# bundle carries real app source and the runbook drops its old caveat.
check "real scaffold source ships" "$EXPORT" '"app/src/main.rs"'
check "runbook drops placeholder caveat" "$(echo "$EXPORT" | python3 -c 'import json,sys; rb=json.load(sys.stdin)["files"]["docs/RUNBOOK.md"]; print("caveat-present" if "scaffold placeholder" in rb else "real-source")')" "real-source"
# Stubbed synthetic-demo jobs must never receive tenant credentials.
check "synthetic demo job omits tenant vault stanza" "$(echo "$EXPORT" | python3 -c 'import json,sys; job=json.load(sys.stdin)["files"]["nomad/job.nomad.hcl"]; print("stanza-present" if "vault {" in job else "stanza-missing")')" "stanza-missing"
# F3: a released app's compliance record embeds the report frozen at
# promotion — the evidence that admitted it — never a lineage re-run.
COMPLIANCE=$(echo "$EXPORT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["files"]["docs/COMPLIANCE.md"])')
check "compliance report frozen at promotion" "$COMPLIANCE" 'frozen at promotion'
check "compliance names the stub"             "$COMPLIANCE" 'STUBBED —'
check "compliance carries HIPAA citations"    "$COMPLIANCE" '45 CFR §164.312(b)'
# The adversarial broken scaffold (#3) is cargo-test-only fixture data:
# it must never register as a pack or reach any API surface.
check "adversarial fixture is not a shipped pack" "$(get /api/packs | python3 -c 'import json,sys; p=json.load(sys.stdin)["packs"]; print("absent" if not any("broken" in x["id"] for x in p) and len(p)==17 else "present")')" "absent"

echo "-- rollback destroys the allocation"
BACK=$(post "/api/apps/$ID/rollback")
check "back to sandbox"   "$BACK" '"stage":"sandbox"'
check "synthetic again"   "$BACK" '"kind":"synthetic"'
PROOF_BACK="$BACK"
if [[ "$PROOF_ID" != "$ID" ]]; then
  # Reproduce a restart/retry boundary with real providers: Vault cleanup
  # succeeded outside this process, while the durable app record still owns
  # the original lease handle. The platform must prove role absence, skip a
  # second revoke, stop Nomad, and finish the rollback truthfully.
  if [[ -n "${VAULT_ADDR:-}" && -n "${CONTROL_DB_URL:-}" && -n "$PSQL" \
    && -n "$PLAT_LEASE" && -n "$PLAT_VUSER" ]]; then
    vault_api PUT "/sys/leases/revoke" "{\"lease_id\":\"$PLAT_LEASE\"}" >/dev/null
    check "externally revoked platform role is absent before retry" \
      "$($PSQL "$CONTROL_DB_URL" -tAc "SELECT count(*) FROM pg_roles WHERE rolname='$PLAT_VUSER'")" "0"
  fi
  PROOF_BACK=$(post "/api/apps/$PROOF_ID/rollback")
  check "infrastructure app returns to sandbox" "$PROOF_BACK" '"stage":"sandbox"'
fi
if [[ -n "${NOMAD_ADDR:-}" ]]; then
  NSTOP=$(curl -s "$NOMAD_ADDR/v1/job/$PROOF_ID?namespace=$NS")
  check "nomad job stopped on rollback" "$NSTOP" '"Stop":true'
else
  echo "  skipped (no nomad): nomad job stopped on rollback"
fi
# #9: revocation observed — the platform's own lease died with the
# allocation: the role Vault created for it is gone from pg_roles (the
# control plane already proved login-failure before recording the event).
if [[ -n "${VAULT_ADDR:-}" && -n "${CONTROL_DB_URL:-}" && -n "$PSQL" && -n "$PLAT_VUSER" ]]; then
  check "platform-issued role dropped from pg_roles on rollback" \
    "$($PSQL "$CONTROL_DB_URL" -tAc "SELECT count(*) FROM pg_roles WHERE rolname='$PLAT_VUSER'")" "0"
else
  echo "  skipped (no vault+control-db+psql): platform-issued role dropped on rollback"
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
  PROOF_AUDIT=$(get "/api/apps/$PROOF_ID/audit")
  check "audit has nomad.job_submitted" "$PROOF_AUDIT" '"nomad.job_submitted"'
  check "audit has nomad.job_stopped"   "$PROOF_AUDIT" '"nomad.job_stopped"'
else
  echo "  skipped (no nomad): audit has nomad.job_submitted, nomad.job_stopped"
fi
if [[ -n "${VAULT_ADDR:-}" ]]; then
  PROOF_AUDIT=$(get "/api/apps/$PROOF_ID/audit")
  check "audit has vault.transit_verified" "$PROOF_AUDIT" '"vault.transit_verified"'
  check "audit has vault.policy mounted or reused" \
    "$PROOF_AUDIT$(vault_api GET "/sys/policies/acl/$NS")" "transit/encrypt/$NS"
else
  echo "  skipped (no vault): audit has vault.transit_verified, vault.policy_mounted"
fi
if [[ -n "${VAULT_ADDR:-}" && -n "${CONTROL_DB_URL:-}" ]]; then
  PROOF_AUDIT=$(get "/api/apps/$PROOF_ID/audit")
  check "audit has vault.db_creds_issued" "$PROOF_AUDIT" '"vault.db_creds_issued"'
  if [[ -n "$PLAT_LEASE" && -n "$PLAT_VUSER" ]]; then
    check "audit has verified already-revoked retry" \
      "$PROOF_AUDIT" '"vault.lease_revocation_already_verified"'
  else
    check "audit has vault.lease_revoked" "$PROOF_AUDIT" '"vault.lease_revoked"'
  fi
else
  echo "  skipped (no vault+control-db): audit has vault.db_creds_issued, vault.lease_revoked"
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
# #9: no password in the platform audit export — structurally guaranteed
# (hashi::DbLease quarantines it; the cargo test greps an export), and
# spot-checked here with the one dynamic password this test does hold.
if [[ -n "${VAULT_ADDR:-}" && -n "${CONTROL_DB_URL:-}" ]]; then
  check "db_creds audit rides the no-password label" \
    "$EXPORT_STREAM" 'password held in memory only, never recorded'
  if [[ -n "$SIB_PASS" ]]; then
    check "no dynamic db password in the audit export" \
      "$(echo "$EXPORT_STREAM" | grep -c "$SIB_PASS" || true)" "0"
  fi
else
  echo "  skipped (no vault+control-db): no-password label + password absent from export"
fi
COMPLIANCE8=$(get "/api/apps/$ID/export" | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["files"]["docs/COMPLIANCE.md"])')
check "ejected compliance keeps the doctor's words" "$COMPLIANCE8" "$WORDS"
if [[ -n "${AUDIT_FILE:-}" && -r "$AUDIT_FILE" ]]; then
  check "archive has the registration probe line" "$(cat "$AUDIT_FILE")" '"audit.sink_probe"'
  check "archive holds app.promoted durably"      "$(cat "$AUDIT_FILE")" '"app.promoted"'
  check "archive carries the hmac form"           "$(cat "$AUDIT_FILE")" 'hmac-sha256:'
  check "archive hides the words" "$(grep -c "$WORDS" "$AUDIT_FILE" || true)" "0"
  if [[ -n "$SIB_PASS" ]]; then
    check "archive holds no dynamic db password (#9)" \
      "$(grep -c "$SIB_PASS" "$AUDIT_FILE" || true)" "0"
  fi
else
  echo "  skipped (no AUDIT_FILE): probe line, app.promoted, hmac form, no plaintext in archive"
fi

echo "-- identity & tenancy (#10): second tenant, roles, and honest auth modes"
# The SAME instance serves both tenants: dr-park (lakeside clinician) works
# with an explicit token — tenancy comes from the principal, never the body.
PARK_TOKEN="dev-token-park"; STAFF_TOKEN="dev-token-rivera"
PARK=$(post /api/apps '{"prompt":"a referral tracker for cardiology consults","pack":"hypertension-tracker","name":"lakeside referrals"}' "$PARK_TOKEN")
PARK_ID=$(echo "$PARK" | python3 -c 'import json,sys; print(json.load(sys.stdin)["app"]["id"])')
check "second tenant app lands in lakeside (from the principal)" "$PARK" '"tenant":"lakeside"'
# Cross-tenant reads/promotes answer 404 — existence is never disclosed.
check "cross-tenant GET is 404"     "$(code GET "/api/apps/$PARK_ID" "$TOKEN")" "404"
check "cross-tenant promote is 404" "$(code POST "/api/apps/$PARK_ID/promote" "$TOKEN" '{}')" "404"
check "cross-tenant audit view is 404 (plaintext boundary keyed to the principal)" \
  "$(code GET "/api/apps/$PARK_ID/audit" "$TOKEN")" "404"
check "lists are tenant-scoped" \
  "$(get /api/apps | grep -c "\"$PARK_ID\"" || true)" "0"
check "the owning tenant still sees its app" "$(get /api/apps "$PARK_TOKEN")" "\"$PARK_ID\""
# Roles: staff build in-tenant but cannot promote/co-sign (403, role known
# in-tenant so disclosure is fine) or export the platform audit.
check "staff promote is 403"              "$(code POST "/api/apps/$ID/promote" "$STAFF_TOKEN" '{}')" "403"
check "staff platform audit export is 403" "$(code GET /api/audit/export "$STAFF_TOKEN")" "403"
# Every denial is on the record with the REAL principal ids as actors.
IDEXPORT=$(get /api/audit/export)
check "cross-tenant denial audited"  "$IDEXPORT" '"auth.cross_tenant_denied"'
check "role denial audited"          "$IDEXPORT" '"auth.role_denied"'
check "denied clinician is the actor" "$IDEXPORT" '"actor":"dr-osei"'
check "second tenant actor is real"   "$IDEXPORT" '"actor":"dr-park"'
check "staff actor is real"           "$IDEXPORT" '"actor":"ms-rivera"'

if [[ -n "$SERVER_PID" && -z "${IDENTITIES_FILE:-}" ]]; then
  # Dev fallback (this self-booted instance has no IDENTITIES_FILE): a
  # request with NO header still works — the zero-config UI stays alive —
  # and the audit trail confesses it.
  NOAUTH=$(curl -s "$BASE/api/apps")
  check "headerless request works in dev (fallback keeps the UI alive)" "$NOAUTH" '"apps"'
  check "dev fallback confessed in the audit stream" "$(get /api/audit/export)" '"auth.dev_fallback"'
  check "unknown token is 401 even in dev" "$(code GET /api/apps wrong-token)" "401"
else
  echo "  skipped (IDENTITIES_FILE set or remote instance): dev fallback + confession"
fi

if [[ -n "$SERVER_PID" ]]; then
  # Strict mode + session idle, proven on a second instance booted the way
  # staging boots (IDENTITIES_FILE set; 1s idle so expiry is observable).
  SBASE="http://127.0.0.1:39001"
  # Isolated in-memory instance: empty AUDIT_FILE/CONTROL_DB_URL/NOMAD/VAULT
  # so only the identity behavior differs from stock dev.
  APP_BIND=127.0.0.1:39001 IDENTITIES_FILE=staging/identities.hcl SESSION_IDLE_SECS=1 \
    AUDIT_FILE= CONTROL_DB_URL= NOMAD_ADDR= VAULT_ADDR= VAULT_TOKEN= \
    "${CARGO_TARGET_DIR:-target}/debug/rust-proof-service" >/dev/null 2>&1 &
  STRICT_PID=$!
  for _ in $(seq 1 50); do
    curl -sf "$SBASE/health" >/dev/null 2>&1 && break
    sleep 0.1
  done
  scode() { curl -s -o /dev/null -w '%{http_code}' ${2:+-H "authorization: Bearer $2"} "$SBASE$1"; }
  check "strict: missing token is 401"  "$(scode /api/apps)" "401"
  check "strict: invalid token is 401"  "$(scode /api/apps wrong-token)" "401"
  check "strict: declared token is 200" "$(scode /api/apps "$TOKEN")" "200"
  check "strict: health stays open"     "$(scode /health)" "200"
  sleep 2
  check "session idle past SESSION_IDLE_SECS is 401 (platform auto-logoff)" \
    "$(scode /api/apps "$TOKEN")" "401"
  check "expiry audited with the principal as actor" \
    "$(curl -s -H "authorization: Bearer $TOKEN" "$SBASE/api/audit/export")" '"auth.session_expired"'
  kill "$STRICT_PID" 2>/dev/null || true; STRICT_PID=""
else
  echo "  skipped (remote instance): strict 401s + session idle expiry"
fi

echo
echo "== $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
