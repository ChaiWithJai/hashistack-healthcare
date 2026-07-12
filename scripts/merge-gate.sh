#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

FULL=false
BASE="${MERGE_BASE:-origin/main}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --full) FULL=true ;;
    --base) shift; BASE="${1:?--base needs a git ref}" ;;
    *) echo "usage: $0 [--full] [--base <git-ref>]" >&2; exit 2 ;;
  esac
  shift
done

git rev-parse --verify "$BASE" >/dev/null

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

{
  git diff --name-only "$BASE" --
  git ls-files --others --exclude-standard
} | sed '/^$/d' | sort -u > "$tmp/files"

file_count="$(wc -l < "$tmp/files" | tr -d ' ')"
tracked_lines="$(git diff --numstat "$BASE" -- | awk '{a+=$1; d+=$2} END {print a+d+0}')"
untracked_lines=0
while IFS= read -r path; do
  if ! git ls-files --error-unmatch "$path" >/dev/null 2>&1 && [[ -f "$path" ]]; then
    lines="$(wc -l < "$path" 2>/dev/null || echo 0)"
    untracked_lines=$((untracked_lines + lines))
  fi
done < "$tmp/files"
line_count=$((tracked_lines + untracked_lines))

echo "merge scope against $BASE: $file_count files, $line_count changed lines"

if rg -n '(^|/)(target|node_modules|\.playwright-cli|output)/' "$tmp/files"; then
  echo "error: temporary build or browser output is part of the change" >&2
  exit 1
fi

if (( file_count > 20 || line_count > 1500 )); then
  if [[ "${ALLOW_LARGE_PR:-}" != "1" ]]; then
    echo "error: scope exceeds the default 20 file or 1500 line budget" >&2
    echo "split the PR or rerun with ALLOW_LARGE_PR=1 after documenting why" >&2
    exit 1
  fi
  echo "warning: large PR override is active"
fi

echo "checking platform"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

echo "checking every runnable pack"
while IFS= read -r manifest; do
  pack="$(basename "$(dirname "$(dirname "$manifest")")")"
  echo "  $pack"
  cargo fmt --manifest-path "$manifest" --check
  cargo test --manifest-path "$manifest" --quiet
done < <(find packs -path '*/scaffold/Cargo.toml' -type f | sort)

echo "checking exported starter source budgets"
while IFS= read -r source; do
  bytes="$(wc -c < "$source" | tr -d ' ')"
  if (( bytes > 76800 )); then
    echo "error: $source is ${bytes} bytes, over the 75 KB default" >&2
    exit 1
  fi
done < <(find packs -path '*/scaffold/src/main.rs' -type f | sort)

echo "checking local staging pressure path"
make staging

if [[ "$FULL" == true ]]; then
  echo "checking all platform and artifact scenarios"
  scripts/evals.sh
fi

echo "merge gate passed"
