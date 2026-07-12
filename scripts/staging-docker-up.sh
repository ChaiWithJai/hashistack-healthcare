#!/usr/bin/env bash
# Docker Desktop staging with a real Nomad client allocation.
set -euo pipefail
cd "$(dirname "$0")/.."

network=hashistack-local
containers=(hashistack-nomad hashistack-vault hashistack-postgres)

if [[ "${1:-}" == "down" ]]; then
  for name in "${containers[@]}"; do docker rm -f "$name" 2>/dev/null || true; done
  docker network rm "$network" 2>/dev/null || true
  exit 0
fi

for name in "${containers[@]}"; do docker rm -f "$name" 2>/dev/null || true; done
docker network rm "$network" 2>/dev/null || true
docker network create "$network" >/dev/null
mkdir -p .staging/logs

docker build -q -t hashistack-healthcare-client:local \
  -f staging/client-image/Dockerfile . >/dev/null

docker run -d --name hashistack-postgres --network "$network" -p 5433:5432 \
  -e POSTGRES_USER=staging -e POSTGRES_PASSWORD=staging-pg \
  -e POSTGRES_DB=control postgres:16 >/dev/null

docker run -d --name hashistack-vault --network "$network" -p 8200:8200 \
  --cap-add IPC_LOCK -v "$PWD/.staging/logs:/vault/logs" \
  -e VAULT_DEV_ROOT_TOKEN_ID=staging-root \
  -e VAULT_DEV_LISTEN_ADDRESS=0.0.0.0:8200 \
  hashicorp/vault:1.17.6 server -dev >/dev/null

shared="${TMPDIR:-/tmp}/hashistack-nomad-docker"
rm -rf "$shared"
mkdir -p "$shared/data" "$shared/alloc"
nomad_config=$(mktemp "${TMPDIR:-/tmp}/hashistack-nomad-client.XXXXXX.hcl")
cp "$PWD/staging/nomad-docker-client.hcl" "$nomad_config"
trap 'rm -f "$nomad_config"' EXIT
docker run -d --name hashistack-nomad --network "$network" -p 4646:4646 \
  --privileged --cgroupns=host --cgroup-parent=nomad-proof \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v "$shared:$shared" \
  -v "$nomad_config:/etc/nomad.d/client.hcl:ro" \
  -e VAULT_ADDR=http://hashistack-vault:8200 -e VAULT_TOKEN=staging-root \
  -e NOMAD_SKIP_DOCKER_IMAGE_WARN=1 \
  hashicorp/nomad:1.8.4 agent -dev -bind=0.0.0.0 \
  -data-dir="$shared/data" -alloc-dir="$shared/alloc" \
  -config=/etc/nomad.d/client.hcl -log-level=INFO >/dev/null

ready=0
for _ in $(seq 1 60); do
  # The Postgres image briefly accepts connections during initdb and then
  # restarts. Require its post-init marker before accepting pg_isready so a
  # control plane cannot attach to that transient server and lose its socket.
  if docker logs hashistack-postgres 2>&1 | grep -q 'PostgreSQL init process complete' &&
     curl -sf http://127.0.0.1:8200/v1/sys/health >/dev/null &&
     curl -sf http://127.0.0.1:4646/v1/agent/health >/dev/null &&
     PGPASSWORD=staging-pg pg_isready -h 127.0.0.1 -p 5433 -U staging -d control >/dev/null; then
    ready=1
    break
  fi
  sleep 1
done
if [[ "$ready" != "1" ]]; then
  echo "Docker staging services did not become ready" >&2
  exit 1
fi

vault() {
  docker exec -e VAULT_ADDR=http://127.0.0.1:8200 \
    -e VAULT_TOKEN=staging-root hashistack-vault vault "$@"
}
vault secrets enable transit >/dev/null
vault audit enable file file_path=/vault/logs/vault-audit.log >/dev/null
vault secrets enable database >/dev/null
vault write database/config/staging-postgres \
  plugin_name=postgresql-database-plugin allowed_roles=tenant-app \
  'connection_url=postgresql://{{username}}:{{password}}@hashistack-postgres:5432/control?sslmode=disable' \
  username=staging password=staging-pg >/dev/null
vault write database/roles/tenant-app db_name=staging-postgres \
  'creation_statements=CREATE ROLE "{{name}}" WITH LOGIN PASSWORD '\''{{password}}'\'' VALID UNTIL '\''{{expiration}}'\''; GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO "{{name}}";' \
  default_ttl=1h max_ttl=2h >/dev/null

cat <<'OUT'
Docker staging is ready.

Start the control plane with these environment values, then run the proof:

  export NOMAD_ADDR=http://127.0.0.1:4646
  export NOMAD_STAGING_IMAGE=hashistack-healthcare-client:local
  export NOMAD_REQUIRE_ALLOCATION=1
  export VAULT_ADDR=http://127.0.0.1:8200 VAULT_TOKEN=staging-root
  export CONTROL_DB_URL=postgres://staging:staging-pg@127.0.0.1:5433/control
  export AUDIT_FILE=.staging/logs/audit-docker.jsonl
  export IDENTITIES_FILE=staging/identities.hcl SESSION_IDLE_SECS=900
  APP_BIND=127.0.0.1:39100 cargo run &
  export CONTROL_PLANE_PID=$!

In another terminal:

  scripts/pressure-test.sh http://127.0.0.1:39100

Stop the infrastructure with scripts/staging-docker-up.sh down.
OUT
