#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

MODEL_URL="${MODEL_URL:-http://127.0.0.1:8081}"
THRESHOLD_REQUEST="${1:-route pain scores of 6 or higher to the practice inbox}"
EXPECTED_THRESHOLD="${2:-6}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/generative-fill.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
cp -R packs/post-op-monitor "$WORK/post-op-monitor"

prompt=$(python3 - "$THRESHOLD_REQUEST" <<'PY'
import json, sys
print(json.dumps({
  "model": "local",
  "temperature": 0,
  "max_tokens": 80,
  "messages": [
    {
      "role": "system",
      "content": "You perform one bounded generative fill in reviewed Rust source. Extract the new threshold explicitly requested by the user. The current value is context only and must not override the requested value. Return exactly one JSON object and no prose."
    },
    {
      "role": "user",
      "content": "Current pain escalation threshold: 7. Requested change: " + sys.argv[1] + ". Output schema: {\"pain_escalation_threshold\": INTEGER}. Allowed range: 4 through 9."
    }
  ]
}))
PY
)

response=$(curl -fsS "$MODEL_URL/v1/chat/completions" \
  -H 'content-type: application/json' -d "$prompt")
threshold=$(printf '%s' "$response" | python3 -c '
import json,sys
r=json.load(sys.stdin)
text=r["choices"][0]["message"]["content"].strip()
if text.startswith("```"):
    text=text.split("\n",1)[1].rsplit("```",1)[0]
v=json.loads(text)
assert set(v)=={"pain_escalation_threshold"}
n=v["pain_escalation_threshold"]
assert type(n) is int and 4 <= n <= 9
print(n)')
if [[ "$threshold" != "$EXPECTED_THRESHOLD" ]]; then
  printf 'generative-fill rejected: expected threshold=%s, model returned=%s\n' \
    "$EXPECTED_THRESHOLD" "$threshold" >&2
  exit 1
fi

file="$WORK/post-op-monitor/scaffold/src/main.rs"
perl -pi -e "s/const PAIN_ESCALATION_THRESHOLD: u8 = 7;/const PAIN_ESCALATION_THRESHOLD: u8 = $threshold;/" "$file"
grep -q "const PAIN_ESCALATION_THRESHOLD: u8 = $threshold;" "$file"

(
  cd "$WORK/post-op-monitor/scaffold"
  cargo fmt --check
  cargo test --quiet
)

printf 'generative-fill passed: request=%q threshold=%s model=%s\n' \
  "$THRESHOLD_REQUEST" "$threshold" "$MODEL_URL"
