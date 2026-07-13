#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

python3 - <<'PY'
from pathlib import Path
import re
import sys

files = [Path("README.md"), *Path("docs").rglob("*.md")]
link_pattern = re.compile(r"(?<!!)\[[^]]+\]\(([^)]+)\)")
command_pattern = re.compile(r"`((?:scripts|terraform)/[^ `]+)")
errors = []

for source in files:
    text = source.read_text(encoding="utf-8")
    for raw in link_pattern.findall(text):
        target = raw.split("#", 1)[0]
        if not target or "://" in target or target.startswith("mailto:"):
            continue
        resolved = (source.parent / target).resolve()
        if not resolved.exists():
            errors.append(f"{source}: broken link {raw}")

    for command in command_pattern.findall(text):
        target = Path(command.rstrip(".,;)"))
        if not target.exists():
            errors.append(f"{source}: missing command or path {command}")

if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)

print(f"documentation check passed for {len(files)} Markdown files")
PY
