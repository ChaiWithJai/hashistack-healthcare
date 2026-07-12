#!/usr/bin/env bash
# Advance an already-provisioned single-host studio to one reviewed commit.
set -euo pipefail

host="${1:?usage: scripts/single-host-release.sh root@HOST COMMIT_SHA}"
ref="${2:?usage: scripts/single-host-release.sh root@HOST COMMIT_SHA}"

if [[ ! "$ref" =~ ^[0-9a-f]{40}$ ]]; then
  echo "release ref must be a full 40-character lowercase commit SHA" >&2
  exit 2
fi

ssh "$host" bash -s -- "$ref" <<'REMOTE'
set -euo pipefail
ref="$1"
repo=/opt/hashistack-healthcare
cd "$repo"
git fetch --depth 1 origin "$ref"
test "$(git rev-parse FETCH_HEAD)" = "$ref"
git checkout --detach FETCH_HEAD
docker compose --env-file /etc/hashistack-studio.env up -d --build --wait
test "$(git rev-parse HEAD)" = "$ref"
REMOTE

base="${STAGING_BASE_URL:-http://${host#*@}:3000}"
"$(dirname "$0")/single-host-remote-proof.sh" "$base"
printf 'released %s to %s and remote checks passed\n' "$ref" "$host"
