# Ops Runbook

## Start
```bash
cargo run
```

## Smoke Check
```bash
curl http://127.0.0.1:3000/health
```

## Troubleshooting
- If Rust is missing, install stable Rust and rerun CI commands.
- If the service fails to bind, check whether port 3000 is already in use.
- If tests fail, record the failure and the smallest fix in the evidence index.
