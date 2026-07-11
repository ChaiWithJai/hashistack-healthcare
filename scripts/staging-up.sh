#!/usr/bin/env bash
# staging-up.sh — the virtual HashiStack staging environment (#2, #7).
#
# Boots, on one machine with no cloud account:
#   1. a real Vault dev server (with the transit engine enabled),
#   2. a real Nomad dev agent,
#   3. a real Postgres control DB on 127.0.0.1:5433 (#7) — portable
#      binaries from theseus-rs/postgresql-binaries, apt fallback,
#      skipped entirely when CONTROL_DB_URL is already exported
#      (e.g. a CI service container),
#   4. the control plane, wired to all three via NOMAD_ADDR / VAULT_ADDR /
#      CONTROL_DB_URL.
#
# Binaries are downloaded once from releases.hashicorp.com, version-pinned
# and checksum-verified, into .staging/bin. Everything runs in the
# background with logs in .staging/logs; re-running is idempotent.
#
# Usage:
#   scripts/staging-up.sh          # download if needed, boot everything
#   scripts/staging-up.sh down     # stop everything this script started
#   scripts/staging-up.sh --models # (stub) the staging inference tier
#
# Then drive the whole workflow against real infrastructure:
#   NOMAD_ADDR=http://127.0.0.1:4646 VAULT_ADDR=http://127.0.0.1:8200 \
#     scripts/pressure-test.sh http://127.0.0.1:39100
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- --models: the staging inference tier (decision 0002) — STUB ----
# TODO(decision 0002): this step will fetch pinned, checksum-verified GGUF
# weights (Liquid LFM2.5 family, stitched per task class) into
# .staging/models, start llama.cpp server(s) as local processes/Nomad jobs,
# and export LOCAL_MODEL_URL for the control plane. The router gains no
# config — environments differ only in what the URL points at. Deliberately
# a documented no-op today: no model weights are downloaded.
if [[ "${1:-}" == "--models" ]]; then
  echo "--models is a documented stub (decision 0002 — inference test tiers):"
  echo "  will fetch pinned GGUF weights -> .staging/models/"
  echo "  will start llama.cpp server(s) on 127.0.0.1 and print LOCAL_MODEL_URL"
  echo "  until then: tests use in-process loopback mocks; staging runs the"
  echo "  rules-only ladder unless you export LOCAL_MODEL_URL yourself."
  exit 0
fi

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

# ---- postgres control DB (#7) ----
# CONTROL_DB_URL already exported (a CI service container, a managed DB):
# trust it and boot nothing. Otherwise boot a local postgres on :5433 —
# preferred: portable, checksum-verified binaries from
# github.com/theseus-rs/postgresql-binaries (single tarball, no root);
# fallback: apt-get postgresql when the release is unreachable.
PG_PORT=5433
PG_DATA="$STAGING_DIR/pgdata"
PG_LOG="$LOG_DIR/postgres.log"
PG_USER="staging"
PG_DB="control"

if [[ -n "${CONTROL_DB_URL:-}" ]]; then
  echo "== control DB: using exported CONTROL_DB_URL (not booting postgres)"
else
  export CONTROL_DB_URL="postgres://$PG_USER@127.0.0.1:$PG_PORT/$PG_DB"
  PG_VERSION="16.4.0"
  PG_DIR="$STAGING_DIR/postgres"
  PG_BIN="$PG_DIR/bin"

  if [[ ! -x "$PG_BIN/initdb" ]]; then
    pg_arch="$(uname -m)" # x86_64 | aarch64 — matches the release asset names
    asset="postgresql-${PG_VERSION}-${pg_arch}-unknown-linux-gnu.tar.gz"
    url="https://github.com/theseus-rs/postgresql-binaries/releases/download/${PG_VERSION}/${asset}"
    echo "== downloading portable postgres $PG_VERSION ($pg_arch)"
    if curl -fsSL --retry 3 --retry-delay 2 -o "$STAGING_DIR/$asset" "$url" &&
       curl -fsSL --retry 3 --retry-delay 2 -o "$STAGING_DIR/$asset.sha256" "$url.sha256"; then
      want="$(cut -d' ' -f1 <"$STAGING_DIR/$asset.sha256")"
      got="$(sha256sum "$STAGING_DIR/$asset" | cut -d' ' -f1)"
      if [[ "$want" != "$got" ]]; then
        echo "ERROR: checksum mismatch for $asset — refusing to run it" >&2
        exit 1
      fi
      mkdir -p "$PG_DIR"
      tar -xzf "$STAGING_DIR/$asset" --strip-components=1 -C "$PG_DIR"
      rm -f "$STAGING_DIR/$asset" "$STAGING_DIR/$asset.sha256"
    else
      # (b) apt fallback — the portable release was unreachable.
      echo "== portable postgres unreachable — falling back to apt-get"
      SUDO=""
      [[ "$(id -u)" -ne 0 ]] && SUDO="sudo"
      $SUDO apt-get update -qq && $SUDO apt-get install -y -qq postgresql >/dev/null
      apt_bin="$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -1)"
      if [[ -z "$apt_bin" ]]; then
        echo "ERROR: apt install left no postgres binaries under /usr/lib/postgresql" >&2
        exit 1
      fi
      mkdir -p "$PG_DIR"
      ln -sfn "$apt_bin" "$PG_BIN"
    fi
  fi

  # initdb refuses to run as root — in a root container, run postgres as a
  # dedicated system user (GH runners and dev laptops skip this branch).
  PG_RUN=""
  if [[ "$(id -u)" -eq 0 ]]; then
    id -u pgstaging >/dev/null 2>&1 || useradd --system --shell /bin/sh pgstaging
    mkdir -p "$PG_DATA"
    touch "$PG_LOG"
    chown -R pgstaging "$PG_DATA" "$PG_LOG"
    PG_RUN="setpriv --reuid=pgstaging --regid=pgstaging --clear-groups"
  fi

  if [[ ! -f "$PG_DATA/PG_VERSION" ]]; then
    echo "== initdb ($PG_DATA)"
    $PG_RUN "$PG_BIN/initdb" -D "$PG_DATA" -U "$PG_USER" --auth=trust -E UTF8 \
      >"$PG_LOG" 2>&1
  fi
  if ! $PG_RUN "$PG_BIN/pg_ctl" -D "$PG_DATA" status >/dev/null 2>&1; then
    echo "== booting postgres on 127.0.0.1:$PG_PORT"
    $PG_RUN "$PG_BIN/pg_ctl" -D "$PG_DATA" -l "$PG_LOG" \
      -o "-p $PG_PORT -c listen_addresses=127.0.0.1 -k ''" start >/dev/null
  fi
  for _ in $(seq 1 100); do
    $PG_RUN "$PG_BIN/pg_isready" -h 127.0.0.1 -p "$PG_PORT" >/dev/null 2>&1 && break
    sleep 0.2
  done
  $PG_RUN "$PG_BIN/pg_isready" -h 127.0.0.1 -p "$PG_PORT" >/dev/null 2>&1 || {
    echo "ERROR: postgres never became ready — last log lines:" >&2
    tail -n 30 "$PG_LOG" >&2 || true
    exit 1
  }
  head -1 "$PG_DATA/postmaster.pid" >"$RUN_DIR/postgres.pid" 2>/dev/null || true
  # create the control database (idempotent)
  if ! $PG_RUN "$PG_BIN/psql" -h 127.0.0.1 -p "$PG_PORT" -U "$PG_USER" -d postgres -tAc \
      "SELECT 1 FROM pg_database WHERE datname='$PG_DB'" | grep -q 1; then
    $PG_RUN "$PG_BIN/createdb" -h 127.0.0.1 -p "$PG_PORT" -U "$PG_USER" "$PG_DB"
  fi
  echo "   postgres healthy: $CONTROL_DB_URL"
fi

# ---- control plane, wired to all three ----
echo "== building + booting the control plane"
cargo build --quiet
BINARY="${CARGO_TARGET_DIR:-target}/debug/rust-proof-service"
if curl -sf "http://$APP_BIND/health" >/dev/null 2>&1; then
  echo "== control plane already running at http://$APP_BIND"
else
  nohup env APP_BIND="$APP_BIND" \
    NOMAD_ADDR="$NOMAD_ADDR" VAULT_ADDR="$VAULT_ADDR" VAULT_TOKEN="$VAULT_TOKEN" \
    CONTROL_DB_URL="$CONTROL_DB_URL" \
    "$BINARY" >"$LOG_DIR/control-plane.log" 2>&1 &
  echo $! >"$RUN_DIR/control-plane.pid"
fi
wait_for control-plane "http://$APP_BIND/health" "$LOG_DIR/control-plane.log"

echo
echo "== staging is up"
echo "   control plane  http://$APP_BIND    (doctor UI at /)"
echo "   nomad          $NOMAD_ADDR"
echo "   vault          $VAULT_ADDR    (token: $VAULT_TOKEN)"
echo "   control DB     $CONTROL_DB_URL"
echo "   logs           $LOG_DIR/"
echo
echo "pressure-test it (real job registration + transit round-trip + restart survival):"
echo "   NOMAD_ADDR=$NOMAD_ADDR VAULT_ADDR=$VAULT_ADDR CONTROL_DB_URL=$CONTROL_DB_URL \\"
echo "     scripts/pressure-test.sh http://$APP_BIND"
echo "tear down:"
echo "   scripts/staging-up.sh down"
