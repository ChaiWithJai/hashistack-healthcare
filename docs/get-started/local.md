# Run Practice Studio locally

This tutorial takes you through one complete synthetic workflow. It uses Docker
Compose so the same service shape works on macOS, Linux, and a single cloud VM.

## Prerequisites

Install:

- Docker with Compose v2;
- Git;
- `curl`.

Give Docker at least 4 GB of memory. The first build downloads Rust images and
can take several minutes.

## Start the service

From the repository root:

```bash
scripts/single-host-smoke.sh
```

The command builds the service, starts Postgres, waits for health, and drives a
synthetic API proof. It exits only after that proof passes.

Open [http://localhost:3000](http://localhost:3000).

## Complete the clinician flow

1. Choose a starter.
2. Describe a small tool.
3. Ask for one change.
4. Open the release check.
5. Repair the named failure.
6. Publish the synthetic preview.

The browser receives a signed private workspace cookie. Another browser cannot
list or change that workspace. Anonymous export is rejected.

## Check health

```bash
curl -fsS http://127.0.0.1:3000/health
docker compose ps
```

Both the `studio` and `postgres` services should report healthy.

## Stop and restart

Keep the database:

```bash
docker compose down
scripts/single-host-smoke.sh
```

Delete all local Practice Studio data:

```bash
docker compose down --volumes
```

The second command is destructive.

## Run the full HashiStack proof

```bash
scripts/staging-docker-up.sh
```

The script prints the control plane command for a second terminal. That proof
requires Nomad to run a generated application, Vault to issue credentials, and
Postgres to preserve state. Stop it with:

```bash
scripts/staging-docker-up.sh down
```

## Next steps

- [Try hosted staging](hosted-preview.md)
- [Read the command reference](../reference/commands.md)
- [Troubleshoot setup](../troubleshooting.md)
- [Understand access control](../reference/access-control.md)
