# Rust Proof Service

This is the learner-owned proof repo for the Async Backend Product Engineer Rust path. It is modeled after product quickstarts: run the service, prove one behavior, document the product decision, and open a reviewable artifact.

The goal is not to prove that Rust is always the right tool. The goal is to prove when a backend slice deserves owned Rust instead of Supabase, Cloudflare, vendor APIs, or ordinary API glue.

## First Win

Before editing code, write three lines:

1. customer workflow: the real workflow this repo protects,
2. managed default: the hosted or simpler stack you would try first,
3. Rust boundary: the exact reliability, replay, contract, async, traceability, or failure-handling risk that justifies this service.

Use `docs/product-use-case.md` for the decision note.

## Quickstart

```bash
cp env.example .env
cargo run
curl http://127.0.0.1:3000/health
cargo test
```

Expected proof:

- `/health` returns a service status,
- tests pass,
- `docs/product-use-case.md` names the simpler stack and Rust boundary,
- `docs/evidence-index.md` links the command output or CI run.

## Docker Fallback

Use this when the local Rust toolchain is missing or noisy:

```bash
docker compose up --build
```

In another terminal:

```bash
curl http://127.0.0.1:3000/health
```

## Common Commands

```bash
make run
make test
make check
make proof
```

`make proof` prints the files a reviewer should inspect before accepting the assignment.

## Product Workflow

Name the customer workflow this repo protects.

## Managed Default

Name the Supabase, Cloudflare, or API path you would try first.

## Rust Boundary

Name the exact risk that makes owning Rust useful: replay, traceability, async state, correctness, or failure handling.

## Test

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Proof

Link the evidence rows in `docs/evidence-index.md`.

## Review And Support

- Setup friction: open an issue with the `setup-friction` template.
- Lab bug: open an issue with the `lab-bug` template.
- Proof review: open an issue with the `proof-review` template or a pull request.

Reviewers should check the product decision before judging code style.
