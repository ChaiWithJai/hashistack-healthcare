#!/usr/bin/env bash
set -euo pipefail

base_url="${1:-http://127.0.0.1:3000}"
base_url="${base_url%/}"
run_id="$(date +%s)-$$"
work="$(mktemp -d -t practice-gemma-profile-XXXXXX)"
trap 'rm -rf "$work"' EXIT

index=0
while IFS='|' read -r pack task; do
  [[ -n "$pack" ]] || continue
  index=$((index + 1))
  cookie="$work/cookie-$index"
  created="$work/created-$index.json"
  planned="$work/planned-$index.json"
  name="Gemma recipe profile $run_id $index"

  curl -sS --fail-with-body \
    -c "$cookie" -b "$cookie" \
    -H 'content-type: application/json' \
    -H "origin: $base_url" \
    -H 'sec-fetch-site: same-origin' \
    -X POST "$base_url/api/public/session" \
    --data '{}' \
    -o /dev/null

  curl -sS --fail-with-body \
    -c "$cookie" -b "$cookie" \
    -H 'content-type: application/json' \
    -H "origin: $base_url" \
    -H 'sec-fetch-site: same-origin' \
    -X POST "$base_url/api/apps" \
    --data "$(python3 -c 'import json,sys; print(json.dumps({"prompt":sys.argv[1],"pack":sys.argv[2],"name":sys.argv[3]}))' "$task" "$pack" "$name")" \
    -o "$created"

  app_id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["app"]["id"])' "$created")"
  started="$(python3 -c 'import time; print(time.monotonic_ns())')"
  curl -sS --fail-with-body \
    -c "$cookie" -b "$cookie" \
    -H 'content-type: application/json' \
    -H "origin: $base_url" \
    -H 'sec-fetch-site: same-origin' \
    -X POST "$base_url/api/apps/$app_id/workspace/treatments" \
    --data "$(python3 -c 'import json,sys; print(json.dumps({"task":sys.argv[1]}))' "$task")" \
    -o "$planned"
  finished="$(python3 -c 'import time; print(time.monotonic_ns())')"

  python3 - "$pack" "$app_id" "$started" "$finished" "$planned" <<'PY'
import json
import os
import sys

pack, app_id, started, finished, path = sys.argv[1:]
with open(path) as handle:
    body = json.load(handle)

agent = body["plan_agent"]
plan = body["treatment_plan"]
allowed = ["guided-worklist", "event-timeline", "focused-task"]
ids = [treatment["id"] for treatment in plan["treatments"]]
valid = ids == allowed and plan["recommended_treatment_id"] in allowed
result = {
    "pack": pack,
    "app": app_id,
    "elapsed_ms": (int(finished) - int(started)) // 1_000_000,
    "provider": agent["provider"],
    "model": agent["model"],
    "deployment_version": agent.get("deployment_version"),
    "fallback": agent.get("fallback_reason"),
    "recommended": plan["recommended_treatment_id"],
    "treatment_ids": ids,
    "signed_recipe_contract": valid,
}
print(json.dumps(result, separators=(",", ":")))

if not valid:
    raise SystemExit("planner returned a treatment outside the signed recipe contract")
if os.environ.get("REQUIRE_GEMMA") == "1" and agent["provider"] != "digitalocean":
    raise SystemExit("Gemma was required but the deterministic fallback ran")
PY
done <<'CASES'
post-op-monitor|Help a practice review synthetic post-op check-ins in the safest order.
hypertension-tracker|Help a practice review synthetic blood pressure readings and see what changed.
patient-intake|Help staff finish incomplete synthetic intake work without scanning every form.
insurance-verification|Help staff resolve failed synthetic insurance checks before a visit.
patient-portal|Help staff review unread synthetic portal updates and choose the next action.
inbound-scheduling|Help staff work through synthetic scheduling requests with fewer missed details.
outbound-followup|Help staff review synthetic follow-up replies that still need attention.
rpm-wearables|Help staff review unresolved synthetic device readings and confirm the next action.
visit-notes|Help a clinician review unfinished synthetic visit notes before saving them.
deid-local|Help a reviewer inspect unresolved synthetic de-identification work.
CASES
