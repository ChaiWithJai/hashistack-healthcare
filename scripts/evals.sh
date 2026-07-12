#!/usr/bin/env bash
# Platform eval harness — one idempotent command.
#
# Runs the two nested layers over evals/scenarios/*.json and writes the
# portable scorecard baseline to docs/evals/scorecard.{md,json}:
#
#   layer 1 (job-to-be-done): a fresh in-memory control plane per scenario
#     (ports 39200+), driven over real HTTP through the whole
#     describe → iterate → gate → fix → review → promote → eject workflow;
#   layer 2 (artifact): the ejected bundle unpacked, BUILT, and RUN
#     (ports 39300+), then judged with Playwright — renders, does the
#     clinical job, keeps its honesty markers.
#
# Exit code: nonzero only on harness errors or a failing check in a
# scenario marked must_pass. Known gaps (the refusal scenarios, the four
# packs without runnable scaffolds) are expected-fail and exit zero — the
# scorecard is a regression baseline, not a trophy.
#
# Prereqs: cargo, node >= 20, and Playwright with Chromium. In this repo's
# dev container both are preinstalled (/opt/node22, /opt/pw-browsers); in
# CI (.github/workflows/evals.yml) `npm install playwright` +
# `npx playwright install chromium` provide them and the harness detects
# which world it is in.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"

# Worktree-local target dirs: the control plane build shares the repo's,
# ejected apps compile once into .evals/target (both never leave the tree,
# so parallel sibling agents can't cross-contaminate builds).
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
export EVALS_EJECT_TARGET_DIR="${EVALS_EJECT_TARGET_DIR:-$ROOT/.evals/target}"

# Playwright: prefer the preinstalled browser tree; else fall back to the
# default cache (CI installs chromium there via npx playwright install).
export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
if [[ -d /opt/pw-browsers ]]; then
  export PLAYWRIGHT_BROWSERS_PATH="${PLAYWRIGHT_BROWSERS_PATH:-/opt/pw-browsers}"
fi

echo "== evals: building the control plane"
cargo build --quiet

echo "== evals: running the harness"
node "$ROOT/evals/harness/run.mjs"
