# Evidence Index

| Evidence | Status | Link | Reviewer note |
|---|---|---|---|
| decision | done | [docs/product-use-case.md](product-use-case.md), [RFC 0001](rfc/0001-clinician-platform.md) | Managed default (Lovable/Supabase glue) named; Rust boundary = gate engine + audit pipeline |
| service | done | `src/api.rs`, `/health`, [ci.yml](../.github/workflows/ci.yml) | Control plane serves the doctor UI and 15 API routes; README quickstart matches |
| contract | done | [tests/platform_contract.rs](../tests/platform_contract.rs) | Typed gate report; false-pass test: promote returns 409 naming the failing check, app stays sandboxed |
| reliability | done | `gate_blocks_promotion_until_fixed_then_admits_with_cosign`, `rollback_destroys_allocation_and_returns_to_synthetic_data` | Terminal states + rollback path exercised end-to-end through the public API |
| ops | done | [docs/ops-runbook.md](ops-runbook.md) | Full describe→audit workflow drivable from curl; smoke check documented |
| staging | done | [scripts/staging-up.sh](../scripts/staging-up.sh), [staging.yml](../.github/workflows/staging.yml) | Real Nomad dev agent + Vault dev server bootable on one machine; pressure test asserts job registration, transit round-trip, and stop-on-rollback (#2) |
| routing | done | [src/ladder.rs](../src/ladder.rs), [tests/ladder_contract.rs](../tests/ladder_contract.rs), [decision 0001](decisions/0001-agent-routing.md) | Verified escalation ladder rules→local→frontier; pack-declared policy; crash-visible operation rows; mock-tier tests only (decision 0002) (#4) |
| revision | pending | — | Awaiting first design-partner / reviewer feedback round |
| capstone | in progress | [docs/capstone-case-study.md](capstone-case-study.md) | Phase 0 slice done; Phase 1 (owned substrate) not started |
| public | pending | — | Publish after design-partner cohort; limitation to lead with: staging registers real Nomad jobs, but placement stays virtual (no containers run) |
