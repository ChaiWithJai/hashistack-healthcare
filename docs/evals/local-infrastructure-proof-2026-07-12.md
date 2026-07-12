# Local infrastructure proof for July 12, 2026

## Result

The control plane passed 124 of 124 pressure checks against local Nomad,
Vault, and Postgres services.

The test used these versions:

| Service | Version | Runtime |
|---|---|---|
| Nomad | 1.8.4 | Docker Desktop, server and client |
| Vault | 1.17.6 | Docker Desktop, development server |
| Postgres | 16 | Docker Desktop |
| Control plane | Current local Rust build | macOS |

The services used the repository's documented local ports. Nomad used 4646,
Vault used 8200, Postgres used 5433, and the control plane used 39100.

## What the run proved

The platform registered a production pool job with Nomad and stored its
evaluation identifier. Nomad scheduled the generated blood pressure app on a
client, started its Docker container, and published its allocated HTTP port.
The health route returned HTTP 200 with the synthetic data guard enabled.

Vault created the tenant transit key and completed an encrypt and decrypt
round trip. The test rotated the key and decrypted ciphertext created before
the rotation. Vault also mounted and returned the tenant policy.

Vault issued a one hour Postgres credential. The control plane used that
credential for `SELECT 1` before it recorded the lease. The test revoked a
sibling credential and confirmed that login failed and that Postgres removed
the role.

The test killed the control plane with signal 9. After restart, Postgres
restored the live app, allocation, attestation, operations, audit events, and
Vault lease handle.

Rollback stopped the Nomad job, revoked the platform's Vault lease, proved
that Postgres removed the issued role, and returned the app to synthetic data.

The durable audit archive contained the release, Nomad, and Vault events. It
stored the doctor prompt as an HMAC and did not contain either dynamic database
password used by the test.

## Problems found and fixed

The original pressure test used one stubbed post operation app for two
different promises. It correctly released the app only to the synthetic demo
pool, then incorrectly expected a production Nomad job and Vault database
lease for that same app. With infrastructure configured, Nomad returned 404
and the test stopped while parsing the missing job.

The pressure test now keeps those promises separate. The post operation app
proves that labeled stubs remain limited to synthetic data. A separate blood
pressure app has no stubs and proves the production Nomad, Vault, Postgres,
restart, and rollback path.

The repository's `scripts/staging-up.sh` supports Linux checksums and Linux
Postgres packages only. It exits on Apple Silicon macOS. Docker Desktop can
run the pinned Vault and Postgres services without changes.

Nomad development mode first failed because Docker Desktop did not delegate a
cgroup parent. The working container uses the host cgroup namespace and a
dedicated cgroup parent. Nomad also needs the Docker socket and an allocation
directory whose absolute path is shared by macOS, the Nomad container, and the
Docker daemon.

The first client reported no CPU capacity, used datacenter `dc1`, and had no
`role=prod` metadata. The local client configuration now declares 12000 CPU
shares, datacenter `nyc3`, and the production pool metadata.

The first scheduled task exposed two invalid job settings. The Docker driver
does not accept Docker Compose style `tmpfs`. The job now uses Nomad's native
tmpfs mount block. The job also omitted `ports = ["http"]`, so Nomad reserved a
port without asking Docker to publish it. The rendered job now publishes that
port and binds the app to `0.0.0.0:8080`.

## Remaining limits

This Docker Desktop setup is a local proof environment. HashiCorp does not
support running Nomad clients inside Docker for production. A Linux machine or
virtual machine remains the correct production-like test target even though
the local client now executes the allocation.

Vault ran in development mode with a root token. The tenant policy existed and
was checked, but the workload did not authenticate with a policy limited token.

Postgres and Vault data lived in local containers. This run proves the control
flow and recovery logic, not backup retention or restore from an off machine
archive.

The full 78 scenario artifact evaluation was not rerun against this shared
database. That evaluation creates a fresh control plane for each scenario, so
pointing every scenario at one durable database would change its isolation
contract. The artifact scorecard remains the proof for exported app behavior,
and this pressure run is the proof for the infrastructure lifecycle.

## Reproduction command

Start the three services and build the local app image with:

```sh
scripts/staging-docker-up.sh
```

The script prints the control plane environment and start command. After the
control plane is healthy, the proof command is:

```sh
NOMAD_ADDR=http://127.0.0.1:4646 \
VAULT_ADDR=http://127.0.0.1:8200 \
VAULT_TOKEN=staging-root \
CONTROL_DB_URL=postgres://staging:staging-pg@127.0.0.1:5433/control \
AUDIT_FILE=.staging/logs/audit-macos.jsonl \
IDENTITIES_FILE=staging/identities.hcl \
SESSION_IDLE_SECS=900 \
scripts/pressure-test.sh http://127.0.0.1:39100
```

Set `NOMAD_STAGING_IMAGE=hashistack-healthcare-client:local` and
`NOMAD_REQUIRE_ALLOCATION=1` to require container execution and health traffic.
The final line was `124 passed, 0 failed`.
