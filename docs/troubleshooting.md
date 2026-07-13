# Troubleshoot Practice Studio

## Docker is not running

`scripts/single-host-smoke.sh` needs a working Docker daemon.

```bash
docker version
docker compose version
```

Start Docker and run the proof again.

## Port 3000 is already in use

Find the listener:

```bash
lsof -nP -iTCP:3000 -sTCP:LISTEN
```

Stop the old process or stack. Do not kill an unrelated process without
checking its command.

## A container is unhealthy

```bash
docker compose ps
docker compose logs studio
docker compose logs postgres
```

Keep the logs when filing an issue. Remove tokens and patient information.

## Local state causes a repeat failure

First restart without deleting data:

```bash
docker compose down
scripts/single-host-smoke.sh
```

If you want a clean disposable environment, delete the volumes:

```bash
docker compose down --volumes
```

## A GitHub staging preview cannot reach SSH

The DigitalOcean firewall limits port 22 to an operator address. GitHub-hosted
runners use changing addresses and should not receive broad SSH access.

Confirm `DO_STAGING_SSH_HOST`, the Access service token, the tunnel ingress
rule, and the hostname-formatted known-host entry. Use the approved operator
release path until Cloudflare Tunnel is configured. Do not open port 22 to the
internet to make a preview job green.

## The browser cannot claim a workspace

Check that:

- the browser still has the original anonymous workspace cookie;
- the Clerk issuer and authorized party match the current fixed hostname;
- the Rust principal map contains the Clerk subject;
- the environment uses the correct Clerk instance.

Pull request previews do not use Clerk and cannot claim or export.
