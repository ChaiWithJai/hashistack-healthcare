#!/usr/bin/env bash
# staging-up.sh — the virtual HashiStack staging environment (#2).
#
# Boots, on one machine with no cloud account:
#   1. a real Vault dev server (with the transit engine enabled),
#   2. a real Nomad dev agent,
#   3. the control plane, wired to both via NOMAD_ADDR / VAULT_ADDR.
#
# Binaries are downloaded once from releases.hashicorp.com, version-pinned
# and checksum-verified, into .staging/bin. Everything runs in the
# background with logs in .staging/logs; re-running is idempotent.
#
# Usage:
#   scripts/staging-up.sh          # download if needed, boot everything
#   scripts/staging-up.sh down     # stop everything this script started
#
# Then drive the whole workflow against real infrastructure:
#   NOMAD_ADDR=http://127.0.0.1:4646 VAULT_ADDR=http://127.0.0.1:8200 \
#     scripts/pressure-test.sh http://127.0.0.1:39100
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- pinned versions + SHA256 (from releases.hashicorp.com *_SHA256SUMS) ----
NOMAD_VERSION="1.8.4"
VAULT_VERSION="1.17.6"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    OS=linux ARCH=amd64
    NOMAD_SHA="681832b4ffaff0626119420569f117fb7ad1e323d6c929ef3c0bccb432165c6b"
    VAULT_SHA="0cddc1fbbb88583b5ba5b845f9f8fae47c6fb39a6d48cd543c6ba6fd3ac1a669"
    ;;
  Linux-aarch64|Linux-arm64)
    OS=linux ARCH=arm64
    NOMAD_SHA="54c92041133073cd4b642c2530990fdcd3ccca1003507d0c636448385d867147"
    VAULT_SHA="05a48513fd609e26c25d6b6b74005bce3430984fe4161696236cc2226e664f3a"
    ;;
  *)
    echo "ERROR: no pinned nomad/vault checksums for $(uname -s)-$(uname -m)." >&2
    echo "Add this platform's sums from releases.hashicorp.com to staging-up.sh." >&2
    exit 1
    ;;
esac

STAGING_DIR=".staging"
BIN_DIR="$STAGING_DIR/bin"
LOG_DIR="$STAGING_DIR/logs"
RUN_DIR="$STAGING_DIR/run"

export NOMAD_ADDR="${NOMAD_ADDR:-http://127.0.0.1:4646}"
export VAULT_ADDR="${VAULT_ADDR:-http://127.0.0.1:8200}"
export VAULT_TOKEN="${VAULT_TOKEN:-staging-root}"
APP_BIND="${APP_BIND:-127.0.0.1:39100}"

# ---- down: stop everything we started ----
if [[ "${1:-}" == "down" ]]; then
  for pidfile in "$RUN_DIR"/*.pid; do
    [[ -f "$pidfile" ]] || continue
    pid=$(cat "$pidfile")
    kill "$pid" 2>/dev/null && echo "stopped $(basename "$pidfile" .pid) (pid $pid)" || true
    rm -f "$pidfile"
  done
  exit 0
fi

mkdir -p "$BIN_DIR" "$LOG_DIR" "$RUN_DIR"

# ---- download pinned, checksum-verified binaries ----
fetch() { # fetch <product> <version> <sha256>
  local product="$1" version="$2" sha="$3"
  if [[ -x "$BIN_DIR/$product" ]] && "$BIN_DIR/$product" version 2>/dev/null | grep -q "v$version"; then
    return 0
  fi
  local zip="$STAGING_DIR/${product}_${version}_${OS}_${ARCH}.zip"
  local url="https://releases.hashicorp.com/${product}/${version}/${product}_${version}_${OS}_${ARCH}.zip"
  echo "== downloading $product $version ($OS/$ARCH)"
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$zip" "$url"; then
    echo "ERROR: could not download $url" >&2
    echo "Outbound HTTPS to releases.hashicorp.com is required (behind a proxy," >&2
    echo "check HTTPS_PROXY and the CA bundle — never disable TLS verification)." >&2
    exit 1
  fi
  echo "$sha  $zip" | sha256sum -c - >/dev/null || {
    echo "ERROR: checksum mismatch for $zip — refusing to run it" >&2
    exit 1
  }
  unzip -oq "$zip" "$product" -d "$BIN_DIR"
  chmod +x "$BIN_DIR/$product"
  rm -f "$zip"
}

fetch nomad "$NOMAD_VERSION" "$NOMAD_SHA"
fetch vault "$VAULT_VERSION" "$VAULT_SHA"

wait_for() { # wait_for <name> <health-url> <logfile>
  local name="$1" url="$2" log="$3"
  for _ in $(seq 1 100); do
    curl -sf "$url" >/dev/null 2>&1 && { echo "   $name healthy: $url"; return 0; }
    sleep 0.2
  done
  echo "ERROR: $name never became healthy at $url — last log lines:" >&2
  tail -n 30 "$log" >&2 || true
  exit 1
}

# ---- vault dev server + transit engine ----
if curl -sf "$VAULT_ADDR/v1/sys/health" >/dev/null 2>&1; then
  echo "== vault already running at $VAULT_ADDR"
else
  echo "== booting vault $VAULT_VERSION (dev mode)"
  nohup "$BIN_DIR/vault" server -dev \
    -dev-root-token-id="$VAULT_TOKEN" \
    -dev-listen-address="${VAULT_ADDR#http://}" \
    >"$LOG_DIR/vault.log" 2>&1 &
  echo $! >"$RUN_DIR/vault.pid"
fi
wait_for vault "$VAULT_ADDR/v1/sys/health" "$LOG_DIR/vault.log"
# Transit backs the per-tenant encryption keys; enabling twice is a no-op error.
"$BIN_DIR/vault" secrets enable transit >/dev/null 2>&1 || true

# ---- nomad dev agent ----
if curl -sf "$NOMAD_ADDR/v1/agent/health" >/dev/null 2>&1; then
  echo "== nomad already running at $NOMAD_ADDR"
else
  echo "== booting nomad $NOMAD_VERSION (dev mode)"
  nohup "$BIN_DIR/nomad" agent -dev -bind=127.0.0.1 \
    >"$LOG_DIR/nomad.log" 2>&1 &
  echo $! >"$RUN_DIR/nomad.pid"
fi
wait_for nomad "$NOMAD_ADDR/v1/agent/health" "$LOG_DIR/nomad.log"

# ---- control plane, wired to both ----
echo "== building + booting the control plane"
cargo build --quiet
BINARY="${CARGO_TARGET_DIR:-target}/debug/rust-proof-service"
if curl -sf "http://$APP_BIND/health" >/dev/null 2>&1; then
  echo "== control plane already running at http://$APP_BIND"
else
  nohup env APP_BIND="$APP_BIND" \
    NOMAD_ADDR="$NOMAD_ADDR" VAULT_ADDR="$VAULT_ADDR" VAULT_TOKEN="$VAULT_TOKEN" \
    "$BINARY" >"$LOG_DIR/control-plane.log" 2>&1 &
  echo $! >"$RUN_DIR/control-plane.pid"
fi
wait_for control-plane "http://$APP_BIND/health" "$LOG_DIR/control-plane.log"

echo
echo "== staging is up"
echo "   control plane  http://$APP_BIND    (doctor UI at /)"
echo "   nomad          $NOMAD_ADDR"
echo "   vault          $VAULT_ADDR    (token: $VAULT_TOKEN)"
echo "   logs           $LOG_DIR/"
echo
echo "pressure-test it (asserts real job registration + transit round-trip):"
echo "   NOMAD_ADDR=$NOMAD_ADDR VAULT_ADDR=$VAULT_ADDR scripts/pressure-test.sh http://$APP_BIND"
echo "tear down:"
echo "   scripts/staging-up.sh down"
