# Proof Pull Request

## Evidence IDs

- [ ] decision
- [ ] service
- [ ] contract
- [ ] reliability
- [ ] ops
- [ ] revision
- [ ] capstone
- [ ] public

## Product Risk

Name the workflow risk this PR reduces.

## Proof

Link the test, runbook, trace, release, or case-study section a reviewer should inspect.

## Simpler Stack Check

Name when Supabase, Cloudflare, or a vendor API would still be enough.

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
