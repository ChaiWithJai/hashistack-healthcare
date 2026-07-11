# Contributing

This repo is a learner proof trail. Keep changes small, reviewable, and tied to a product risk.

## Before Opening A Pull Request

```bash
make check
```

Update these files with the code change:

- `docs/product-use-case.md`
- `docs/evidence-index.md`
- `docs/ops-runbook.md`

## Review Order

1. Product decision: why not Supabase, Cloudflare, or API glue?
2. Rust boundary: what risk does this service own?
3. Evidence: what command, test, CI run, or screenshot proves the behavior?
4. Limitations: what does this repo not prove yet?
