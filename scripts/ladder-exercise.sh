#!/usr/bin/env bash
# ladder-exercise.sh — drive the verified escalation ladder with a batch of
# iterate instructions and report REAL per-tier numbers (attempts, verdicts,
# reject reasons, wall latency). Written for the first live staging model
# tier (docs/investigations/0003-hermes-local-experiment.md); useful any time
# LOCAL_MODEL_URL points at a real endpoint and you want decision 0001's
# kill-threshold inputs measured instead of asserted.
#
# Usage:
#   scripts/ladder-exercise.sh [BASE_URL] [LABEL]
#   scripts/ladder-exercise.sh http://127.0.0.1:39100 smollm2-135m
#
# Output: one TSV row per instruction (instruction, tiers climbed, verdicts,
# reasons, wall ms) plus a per-tier accept-rate summary — the exact shape the
# decision 0001 staging-metrics section asks for.
set -euo pipefail

BASE="${1:-http://127.0.0.1:39100}"
LABEL="${2:-unlabeled}"

INSTRUCTIONS=(
  "add automatic logoff after 15 minutes idle"
  "remind patients to log their wound photos daily"
  "add a staff triage queue with role-based access for nurses"
  "flag patients whose pain score rises two days in a row and escalate to the surgeon"
  "translate the daily check-in form to Spanish"
  "remove the audit log, it slows the app down"
  "add a dashboard chart of weekly pain trends"
  "let patients export their own recovery data as a PDF"
)

APP=$(curl -s -X POST "$BASE/api/apps" -H 'content-type: application/json' \
  -d '{"prompt":"ladder exercise: post-op recovery tracker","pack":"post-op-monitor","name":"ladder exercise '"$LABEL"'"}')
ID=$(echo "$APP" | python3 -c 'import json,sys; print(json.load(sys.stdin)["app"]["id"])')
echo "app: $ID (label: $LABEL, base: $BASE)"
echo

for instr in "${INSTRUCTIONS[@]}"; do
  t0=$(date +%s%3N)
  curl -s -X POST "$BASE/api/apps/$ID/iterate" -H 'content-type: application/json' \
    -d "$(python3 -c 'import json,sys; print(json.dumps({"instruction": sys.argv[1]}))' "$instr")" >/dev/null
  t1=$(date +%s%3N)
  echo "ran ($((t1-t0)) ms): $instr"
done

echo
echo "== operations record (the ladder's own evidence)"
OPS_JSON=$(mktemp)
trap 'rm -f "$OPS_JSON"' EXIT
curl -s "$BASE/api/apps/$ID/operations" >"$OPS_JSON"
python3 - "$LABEL" "$OPS_JSON" <<'EOF'
import json, sys

data = json.load(open(sys.argv[2]))
label = sys.argv[1]
ops = data.get("operations", data if isinstance(data, list) else [])
tiers = {}
print(f"{'kind':<9} {'status':<9} attempts (tier:verdict[reason] wall-s)")
for op in ops:
    parts = []
    for a in op.get("attempts", []):
        wall = a["finished_at"] - a["started_at"]
        reason = f"[{a['reason']}]" if a.get("reason") else ""
        parts.append(f"{a['tier']}:{a['verdict']}{reason} {wall}s")
        t = tiers.setdefault(a["tier"], {"attempts": 0, "accepted": 0, "reasons": {}})
        t["attempts"] += 1
        if a["verdict"] == "accepted":
            t["accepted"] += 1
        elif a.get("reason"):
            key = a["reason"].split("(")[0]
            t["reasons"][key] = t["reasons"].get(key, 0) + 1
    print(f"{op['kind']:<9} {op['status']:<9} {' -> '.join(parts)}")

print(f"\n== per-tier summary ({label})")
for tier, t in tiers.items():
    rate = 100.0 * t["accepted"] / t["attempts"] if t["attempts"] else 0.0
    reasons = ", ".join(f"{k}x{v}" for k, v in sorted(t["reasons"].items())) or "-"
    print(f"  {tier:<9} attempts={t['attempts']:<3} accepted={t['accepted']:<3} "
          f"accept-rate={rate:5.1f}%  rejects: {reasons}")
EOF

echo
echo "== escalation evidence in the audit stream (last 12 agent events)"
curl -s "$BASE/api/apps/$ID/audit" | python3 -c '
import json, sys
events = json.load(sys.stdin).get("events", [])
agent = [e for e in events if e.get("action","").startswith("agent.")]
for e in agent[-12:]:
    seq, action, detail = e["seq"], e["action"], e["detail"][:110]
    print(f"  seq {seq:>4}  {action:<14} {detail}")'
