#!/usr/bin/env bash
# staging-up.sh — the virtual HashiStack staging environment (#2, #7, #9).
#
# Boots, on one machine with no cloud account:
#   1. a real Vault dev server (transit engine + file audit device enabled),
#   2. a real Nomad dev agent,
#   3. a real Postgres control DB on 127.0.0.1:5433 (#7) — portable
#      binaries from theseus-rs/postgresql-binaries, apt fallback,
#      skipped entirely when CONTROL_DB_URL is already exported
#      (e.g. a CI service container); host auth is scram-sha-256 so
#      password checks are real, not trust-vacuous (#9),
#   4. Vault's database secrets engine wired against that Postgres (#9):
#      connection database/config/staging-postgres + role tenant-app
#      (CRUD grant, default_ttl=1h, max_ttl=2h),
#   5. the control plane, wired to all of it via NOMAD_ADDR / VAULT_ADDR /
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
# Audit broker (#8): staging always runs with the JSONL file sink attached,
# so the broker invariant (no durable audit write, no operation) is live on
# every staging run alongside the control-DB sink.
export AUDIT_FILE="${AUDIT_FILE:-$STAGING_DIR/logs/audit.jsonl}"
# Identity (#10): staging always runs STRICT — the declared registry
# (missing/invalid bearer tokens answer 401, no dev fallback) and the
# platform's own idle auto-logoff (15 min, the platform honoring the same
# auto-logoff gate it demands of generated apps). The tokens in
# staging/identities.hcl are documented Phase 0 dev credentials, same
# spirit as VAULT_TOKEN=staging-root; OIDC replaces the token source.
export IDENTITIES_FILE="${IDENTITIES_FILE:-staging/identities.hcl}"
export SESSION_IDLE_SECS="${SESSION_IDLE_SECS:-900}"

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

# Vault file audit device (#9): enabled from boot — the Vault audit log is
# itself a HIPAA technical-safeguard artifact (RFC 0001). Enabling twice
# errors ("path already in use"), so tolerate that and then ASSERT the
# device is really on — a staging without it must not come up quietly.
VAULT_AUDIT_LOG="$PWD/$LOG_DIR/vault-audit.log"
"$BIN_DIR/vault" audit enable file file_path="$VAULT_AUDIT_LOG" >/dev/null 2>&1 || true
if ! "$BIN_DIR/vault" audit list 2>/dev/null | grep -q '^file/'; then
  echo "ERROR: vault file audit device is not enabled — refusing a staging" >&2
  echo "without its HIPAA audit artifact. See $LOG_DIR/vault.log" >&2
  exit 1
fi

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
# A fixed, documented dev credential like VAULT_TOKEN=staging-root. Host
# auth is tightened to scram-sha-256 below so the #9 "issued creds
# authenticate / revoked creds fail" evidence is a real password check.
PG_PASS="staging-pg"
export PGPASSWORD="$PG_PASS"

if [[ -n "${CONTROL_DB_URL:-}" ]]; then
  echo "== control DB: using exported CONTROL_DB_URL (not booting postgres)"
else
  export CONTROL_DB_URL="postgres://$PG_USER:$PG_PASS@127.0.0.1:$PG_PORT/$PG_DB"
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
  # #9: real password auth. initdb ships trust for host connections; give
  # the superuser its password while trust is still in effect, then tighten
  # pg_hba host lines to scram-sha-256 and reload. Idempotent: an already
  # tightened data dir has no trust host lines and skips the sed+reload.
  $PG_RUN "$PG_BIN/psql" -h 127.0.0.1 -p "$PG_PORT" -U "$PG_USER" -d postgres -q -c \
    "ALTER ROLE $PG_USER WITH PASSWORD '$PG_PASS'"
  if grep -Eq '^host.*trust$' "$PG_DATA/pg_hba.conf"; then
    sed -i -E 's/^(host.*[[:space:]])trust$/\1scram-sha-256/' "$PG_DATA/pg_hba.conf"
    $PG_RUN "$PG_BIN/pg_ctl" -D "$PG_DATA" reload >/dev/null
    echo "   host auth tightened: trust -> scram-sha-256"
  fi
  # create the control database (idempotent)
  if ! $PG_RUN "$PG_BIN/psql" -h 127.0.0.1 -p "$PG_PORT" -U "$PG_USER" -d postgres -tAc \
      "SELECT 1 FROM pg_database WHERE datname='$PG_DB'" | grep -q 1; then
    $PG_RUN "$PG_BIN/createdb" -h 127.0.0.1 -p "$PG_PORT" -U "$PG_USER" "$PG_DB"
  fi
  echo "   postgres healthy: $CONTROL_DB_URL"
fi

# ---- vault database secrets engine, wired to the staging postgres (#9) ----
# Idempotent: enable tolerates "already enabled"; config and role writes
# overwrite in place. The connection uses the control-DB superuser as the
# root credential; the tenant-app role template creates login roles with a
# CRUD grant on the tenant/control DB, default_ttl=1h (the issue's TTL),
# max_ttl=2h. Revocation (default statements) drops the issued role — the
# rollback proof depends on exactly that.
echo "== vault database secrets engine (role tenant-app, 1h TTL)"
DBU="${CONTROL_DB_URL#postgres://}"; DBU="${DBU#postgresql://}"
if [[ "$DBU" != *@* ]]; then
  echo "ERROR: CONTROL_DB_URL has no user@host part — cannot wire vault's" >&2
  echo "database engine. Use postgres://user:password@host:port/db." >&2
  exit 1
fi
DBU_USERINFO="${DBU%%@*}"; DBU_REST="${DBU#*@}"
DBU_USER="${DBU_USERINFO%%:*}"
DBU_PASS=""; [[ "$DBU_USERINFO" == *:* ]] && DBU_PASS="${DBU_USERINFO#*:}"
DBU_HOSTPORT="${DBU_REST%%/*}"
DBU_DB="${DBU_REST#*/}"; DBU_DB="${DBU_DB%%\?*}"
"$BIN_DIR/vault" secrets enable database >/dev/null 2>&1 || true
"$BIN_DIR/vault" write database/config/staging-postgres \
  plugin_name=postgresql-database-plugin \
  allowed_roles="tenant-app" \
  connection_url="postgresql://{{username}}:{{password}}@$DBU_HOSTPORT/$DBU_DB?sslmode=disable" \
  username="$DBU_USER" password="$DBU_PASS" >/dev/null
"$BIN_DIR/vault" write database/roles/tenant-app \
  db_name=staging-postgres \
  creation_statements="CREATE ROLE \"{{name}}\" WITH LOGIN PASSWORD '{{password}}' VALID UNTIL '{{expiration}}'; GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO \"{{name}}\";" \
  default_ttl=1h max_ttl=2h >/dev/null
echo "   database/creds/tenant-app ready (postgres $DBU_HOSTPORT/$DBU_DB)"

# ---- control plane, wired to all of it ----
echo "== building + booting the control plane"
cargo build --quiet
BINARY="${CARGO_TARGET_DIR:-target}/debug/rust-proof-service"
if curl -sf "http://$APP_BIND/health" >/dev/null 2>&1; then
  echo "== control plane already running at http://$APP_BIND"
else
  nohup env APP_BIND="$APP_BIND" \
    NOMAD_ADDR="$NOMAD_ADDR" VAULT_ADDR="$VAULT_ADDR" VAULT_TOKEN="$VAULT_TOKEN" \
    CONTROL_DB_URL="$CONTROL_DB_URL" AUDIT_FILE="$AUDIT_FILE" \
    IDENTITIES_FILE="$IDENTITIES_FILE" SESSION_IDLE_SECS="$SESSION_IDLE_SECS" \
    "$BINARY" >"$LOG_DIR/control-plane.log" 2>&1 &
  echo $! >"$RUN_DIR/control-plane.pid"
fi
wait_for control-plane "http://$APP_BIND/health" "$LOG_DIR/control-plane.log"

echo
echo "== staging is up"
echo "   control plane  http://$APP_BIND    (doctor UI at /)"
echo "   nomad          $NOMAD_ADDR"
echo "   vault          $VAULT_ADDR    (token: $VAULT_TOKEN)"
echo "   vault audit    $VAULT_AUDIT_LOG    (file device — HIPAA artifact, #9)"
echo "   db creds       database/creds/tenant-app (1h TTL, revoked on rollback)"
echo "   control DB     $CONTROL_DB_URL"
echo "   audit archive  $AUDIT_FILE    (broker: memory fallback + file + control-db)"
echo "   identity       $IDENTITIES_FILE    (strict bearer auth — Phase 0 dev tokens; idle ${SESSION_IDLE_SECS}s)"
echo "   logs           $LOG_DIR/"
echo
echo "pressure-test it (real job registration + transit round-trip + restart survival):"
echo "   NOMAD_ADDR=$NOMAD_ADDR VAULT_ADDR=$VAULT_ADDR VAULT_TOKEN=$VAULT_TOKEN \\"
echo "   CONTROL_DB_URL=$CONTROL_DB_URL \\"
echo "   AUDIT_FILE=$AUDIT_FILE \\"
echo "   IDENTITIES_FILE=$IDENTITIES_FILE SESSION_IDLE_SECS=$SESSION_IDLE_SECS \\"
echo "     scripts/pressure-test.sh http://$APP_BIND"
echo "tear down:"
echo "   scripts/staging-up.sh down"
