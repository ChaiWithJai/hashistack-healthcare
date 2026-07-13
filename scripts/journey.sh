#!/usr/bin/env bash
# Journey profiler — one command, one real clinician journey, profiled.
#
# Runs THE flagship journey (Dr. Osei vibe-coding a post-op recovery tracker
# on the fully-real pack) end to end against a freshly booted control plane:
# describe → sandbox UI → iterate → gate review (screenshotted, production
# limitation enforced) → co-sign synthetic demo → eject → the ejected
# bundle compiled, booted, and driven with Playwright. Every step is timed
# (wall ms around the HTTP call / build / boot) and cross-referenced to the
# audit events it produced.
#
#   ./scripts/journey.sh
#   # → docs/evals/journey/journey.md   (the show-anyone narrative)
#   # → docs/evals/journey/journey.json (every number, machine-readable)
#   # → docs/evals/journey/0*.png       (six stage screenshots, <900KB total)
#
# Ports: 39400 (control plane) and 39450 (ejected app) — clear of the eval
# harness (39200/39300) and the staging pressure test (39000+). Prereqs are
# the eval harness's: cargo, node >= 20, Playwright with Chromium (the dev
# container preinstalls /opt/node22 and /opt/pw-browsers; elsewhere
# `npm install playwright && npx playwright install chromium`).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"

# Worktree-local target dirs (parallel sibling agents must not share): the
# control plane shares the repo's, the ejected app compiles into .journey/.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
export JOURNEY_EJECT_TARGET_DIR="${JOURNEY_EJECT_TARGET_DIR:-$ROOT/.journey-target}"

export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
if [[ -d /opt/pw-browsers ]]; then
  export PLAYWRIGHT_BROWSERS_PATH="${PLAYWRIGHT_BROWSERS_PATH:-/opt/pw-browsers}"
fi

echo "== journey: building the control plane"
cargo build --quiet

echo "== journey: profiling the flagship journey"
node "$ROOT/evals/journey/profile.mjs"
