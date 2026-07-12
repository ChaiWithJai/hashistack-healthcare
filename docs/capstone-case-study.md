# Capstone Case Study

## Workflow Protected
A clinician describes a practice tool in natural language and takes it from
prototype to production in front of real patients. The business driver: every
consumer builder fails at the BAA + compliant-deployment wall (the Ong/Antaki
Tier 3 wall), so validated demand stalls exactly where the risk starts.

## Stack Choice
Managed default was Lovable-style building on rented PaaS — right for Tier 1–2,
structurally unable to express the two things Tier 3 needs: a sandbox pool with
no route to production data, and a gate between sandbox and prod. The RFC keeps
PaaS as an export target and puts HashiStack (Nomad/Vault/Terraform/Packer) on
DigitalOcean under BAA underneath — see
[RFC 0001](rfc/0001-clinician-platform.md), alternatives considered.

## Rust Boundary
The gate engine and audit pipeline. A gate verdict is reproducible evidence
(pure functions over a typed app record), the promotion path is unreachable
with a failing check, and the audit stream is append-only with strictly
increasing sequence numbers. These properties are enforced by the type system
and proven by contract tests, not by convention.

## Evidence
- Repo: this repository; CI: `.github/workflows/ci.yml` (fmt, clippy -D warnings, tests)
- Contract tests: [tests/platform_contract.rs](../tests/platform_contract.rs) (9 workflow tests) + tests/proof_contract.rs (original service contract, still green)
- Runbook: [docs/ops-runbook.md](ops-runbook.md) — the whole workflow from curl
- Failure note: promotion with a failing gate returns 409 naming the check; the app record is unchanged (asserted)
- Design + plan: [docs/design/](design/) wireframes, [RFC 0001](rfc/0001-clinician-platform.md), [HashiCorp steering](hashicorp-steering.md)

## Limitation
Phase 0 allocations are simulated in-process and platform state is in-memory:
this proves the workflow contract and the gate semantics, not scheduler
integration. A simpler version (one clinic, one tool, no multi-tenancy) would
not need Rust at all — Supabase plus a checklist would do; the boundary only
pays for itself once untrusted generated code and a shared audit spine enter.

## Transfer
The gate-before-promotion loop transfers to any regulated
generate-then-deploy workflow: SOC 2/PCI gates for fintech internal tools,
privilege screens for legal, FERPA for education. Packs and gates swap;
the control plane and the workflow contract stay.
