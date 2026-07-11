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
#   scripts/staging-up.sh --models # the staging inference tier (decision 0002)
#
# Then drive the whole workflow against real infrastructure:
#   NOMAD_ADDR=http://127.0.0.1:4646 VAULT_ADDR=http://127.0.0.1:8200 \
#     scripts/pressure-test.sh http://127.0.0.1:39100
set -euo pipefail
cd "$(dirname "$0")/.."

# ---- --models: the staging inference tier (decision 0002) ----
# Fetches pinned, sha256-verified GGUF weights into .staging/models and
# starts one llama.cpp server on 127.0.0.1:8081 with an OpenAI-compatible
# /v1/chat/completions. The router gains no config — environments differ
# only in what LOCAL_MODEL_URL points at.
#
# Sourcing honesty (docs/investigations/0003-hermes-local-experiment.md):
# the preferred Liquid LFM2-class weights live on huggingface.co, which many
# sandboxed/proxied environments (including the one this was built in)
# cannot reach. PyPI is the one artifact host that is reachable virtually
# everywhere this repo's substrate runs, so the pinned weights below are
# GGUF files published inside PyPI wheels, verified by sha256 at every hop
# (wheel hash from files.pythonhosted.org, then the extracted/assembled
# .gguf hash). Swap in the HF-hosted LFM2 GGUF (same shape: URL + sha256)
# when your network allows it.
#
#   default served model : SmolLM2-135M-Instruct Q4_1 (Apache-2.0, instruct)
#   also fetched         : gemma-3-270m Q4_K_M (base model — a deliberately
#                          weak rung to exercise escalation, decision 0002)
#   server               : llama-cpp-python[server], version-pinned, in a
#                          venv under .staging/models/venv
models_step() {
  local models_dir=".staging/models" run_dir=".staging/run" log_dir=".staging/logs"
  local port="${LLAMA_PORT:-8081}"
  mkdir -p "$models_dir" "$run_dir" "$log_dir"

  # pinned wheels: pkg==version, wheel sha256, gguf path inside the wheel
  local smol_pkg="llm-smollm2" smol_ver="0.1.2"
  local smol_whl_sha="bcc81830d10ce7d9e76640cad826a4b79ed3e4547c78a0be5c4f2fb0e2448c70"
  local smol_gguf="SmolLM2-135M-Instruct.Q4_1.gguf"
  local smol_gguf_sha="b179c9523d0e6a0f98a330c7562b682750a6f8c8c15e5bc70ea373728110db53"
  # gemma chunks: 4 wheels, each carrying one .partNN of the gguf
  local gemma_shas=(
    "2ce8a8889efc923beb08b9c2c90df07482bad1dfc00af4bc27235f70425773cf"
    "8fb5776bec6f322a3eca852bda800c2c27bfb5ff7f3490760b9514d676a77fe3"
    "94a258d51e6affe7cfdcc461eefb4091a0a72b2f407e0b6ce6832c004a767b72"
    "1bad81ad2dd240a233067f9e19bec22329b8454e76222d065f6585522077be38"
  )
  local gemma_gguf="gemma-3-270m-q4_k_m.gguf"
  local gemma_gguf_sha="a5fd3b62230aa5ec60212297dc9d20eaa70578ac519e00d93a17e44c087a6818"

  pypi_wheel_url() { # pypi_wheel_url <pkg> <version>
    curl -fsSL "https://pypi.org/pypi/$1/$2/json" | python3 -c \
      'import json,sys; [print(u["url"]) for u in json.load(sys.stdin)["urls"] if u["filename"].endswith(".whl")]'
  }
  fetch_wheel() { # fetch_wheel <pkg> <version> <sha256> <dest>
    local url; url=$(pypi_wheel_url "$1" "$2")
    curl -fsSL --retry 3 --retry-delay 2 -o "$4" "$url"
    echo "$3  $4" | sha256sum -c - >/dev/null || {
      echo "ERROR: wheel checksum mismatch for $1==$2 — refusing to use it" >&2; exit 1; }
  }

  # -- weights: SmolLM2-135M-Instruct (served by default) --
  if ! echo "$smol_gguf_sha  $models_dir/$smol_gguf" | sha256sum -c - >/dev/null 2>&1; then
    echo "== fetching $smol_gguf (pinned, via PyPI $smol_pkg==$smol_ver)"
    fetch_wheel "$smol_pkg" "$smol_ver" "$smol_whl_sha" "$models_dir/.smol.whl"
    (cd "$models_dir" && unzip -oq .smol.whl "llm_smollm2/$smol_gguf" && mv "llm_smollm2/$smol_gguf" . && rmdir llm_smollm2 && rm -f .smol.whl)
    echo "$smol_gguf_sha  $models_dir/$smol_gguf" | sha256sum -c - >/dev/null || {
      echo "ERROR: gguf checksum mismatch for $smol_gguf" >&2; exit 1; }
  fi

  # -- weights: gemma-3-270m (kept alongside as the deliberately-weak rung) --
  if ! echo "$gemma_gguf_sha  $models_dir/$gemma_gguf" | sha256sum -c - >/dev/null 2>&1; then
    echo "== fetching $gemma_gguf (pinned, via PyPI gemma3-270m-q4-k-m-gguf-part1..4)"
    local i part parts=()
    for i in 1 2 3 4; do
      part="$models_dir/.gemma.part$i.whl"
      fetch_wheel "gemma3-270m-q4-k-m-gguf-part$i" "1.0.0" "${gemma_shas[$((i-1))]}" "$part"
      (cd "$models_dir" && unzip -oq ".gemma.part$i.whl" "gemma3_270m_q4_k_m_gguf_part$i/data/*")
      parts+=("$models_dir/gemma3_270m_q4_k_m_gguf_part$i/data/gemma-3-270m-q4_k_m.gguf.part0$((i-1))")
    done
    cat "${parts[@]}" > "$models_dir/$gemma_gguf"
    rm -rf "$models_dir"/.gemma.part*.whl "$models_dir"/gemma3_270m_q4_k_m_gguf_part*
    echo "$gemma_gguf_sha  $models_dir/$gemma_gguf" | sha256sum -c - >/dev/null || {
      echo "ERROR: gguf checksum mismatch for $gemma_gguf" >&2; exit 1; }
  fi

  # -- server: llama-cpp-python[server], version-pinned, venv-local --
  if [[ ! -x "$models_dir/venv/bin/python" ]]; then
    echo "== creating venv + installing llama-cpp-python[server]==0.3.16 (CPU build; takes a few minutes)"
    python3 -m venv "$models_dir/venv"
    "$models_dir/venv/bin/pip" install --quiet --no-cache-dir "llama-cpp-python[server]==0.3.16"
  fi

  # -- start (idempotent): OpenAI-compatible /v1/chat/completions --
  local url="http://127.0.0.1:$port"
  local serve="${STAGING_MODEL_FILE:-$smol_gguf}"
  if curl -sf "$url/v1/models" >/dev/null 2>&1; then
    echo "== llama server already running at $url"
  else
    echo "== starting llama server ($serve) at $url"
    nohup "$models_dir/venv/bin/python" -m llama_cpp.server \
      --model "$models_dir/$serve" --host 127.0.0.1 --port "$port" \
      --n_ctx 4096 --n_threads "${LLAMA_THREADS:-$(( $(nproc) > 2 ? $(nproc) - 1 : 1 ))}" \
      >"$log_dir/llama-server.log" 2>&1 &
    echo $! >"$run_dir/llama-server.pid"
    for _ in $(seq 1 300); do
      curl -sf "$url/v1/models" >/dev/null 2>&1 && break
      sleep 0.5
    done
    curl -sf "$url/v1/models" >/dev/null 2>&1 || {
      echo "ERROR: llama server never became healthy — last log lines:" >&2
      tail -n 30 "$log_dir/llama-server.log" >&2 || true; exit 1; }
  fi
  echo
  echo "== staging model tier is up (decision 0002)"
  echo "   serving        $serve"
  echo "   also on disk   $gemma_gguf (STAGING_MODEL_FILE=$gemma_gguf to serve it)"
  echo "   wire the control plane to it (small CPU models need a roomier read timeout):"
  echo "   export LOCAL_MODEL_URL=$url MODEL_HTTP_TIMEOUT_SECS=60"
  echo "   then (re)run: scripts/staging-up.sh   # env is inherited by the control plane"
  echo "   tear down with the rest: scripts/staging-up.sh down"
}
if [[ "${1:-}" == "--models" ]]; then
  models_step
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
