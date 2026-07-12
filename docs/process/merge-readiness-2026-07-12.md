# Merge readiness review for July 12, 2026

## Decision

No open PR is ready to merge under `docs/process/merge-standard.md`.

There are no reviews or approvals on the open chain. PRs 15 through 22 also
fail the `pack-scaffolds (post-op-monitor)` check because the scaffold does not
pass `cargo fmt --check`. The failure is deterministic.

## Current PR chain

The live chain is:

`#1`, `#13`, `#14`, `#15`, `#16`, `#17`, `#19`, `#20`, `#21`, `#22`.

Each PR after 1 targets the previous feature branch. This makes the history
readable, but it does not create independently mergeable changes. Merge or
replace the chain in reviewed groups and retarget each group to `main` after
its dependency lands.

| PR | Status | Main gate |
|---:|---|---|
| 1 | Conditional foundation | Green checks, but 48 files and 6,833 added lines mix UI, control plane, infrastructure, and strategy. Human architecture review and footprint evidence are missing. |
| 13 | Not ready | Model, Nomad, and Vault work occurs under the platform write lock in this revision. Partial external success has incomplete compensation. Three tickets are mixed in one PR. |
| 14 | Conditional synthetic reference | Green and bounded compared with the chain. It is one synthetic, process local pack with a large single source file and no independent review. |
| 15 | Blocked | CI formatting fails. Stubbed gates count as green and can permit production promotion in this revision. |
| 16 | Blocked on the live PR | CI fails on the live head. The integrated local tree now writes RUNNING to the control database before driver work. Non stage writes can still accept degraded database durability. |
| 17 | Blocked on the live PR | CI fails on the live head. The integrated local tree now uses an exact state check before audit compensation. Required archive integrity work remains. |
| 19 | Blocked on the live PR | CI fails on the live head. The integrated local tree revokes a newly issued lease when Nomad submission fails. Rollback still needs an explicit recovery state, and workload identity is not enforcing. |
| 20 | Blocked | CI fails. Static reusable tokens are not production identity. Idle expiry does not revoke the token. A report hash is not a signed attestation. |
| 21 | Blocked | CI fails. The 5,659 line closeout mixes evaluation, auth, refusal, model staging, deploy status, and documentation over the unmerged chain. |
| 22 | Blocked | CI fails. The journey is useful evidence for one post op path, but it does not prove the issue bars for all packs or profiles. |

PR 1, PR 13, and PR 14 report a clean merge state in GitHub. That status only
means Git can combine their branches. It does not satisfy the review, scope,
architecture, or user gates.

## Issue status

| Issue | Current decision |
|---:|---|
| 2 | Keep open until the staging run is attached to a merged workflow and proves the current head. |
| 3 | Keep open until stubs block real data and evidence coverage matches every production claim. |
| 4 | Keep open until the chosen model path proves a doctor task at its declared tier. The integrated local tree now makes the operation record durable before work when the control database is configured. |
| 5 | Keep open. Seventeen runnable starters exist locally, but the full pack layout, clinical citations, and registry signature boundary are not proven for every pack. |
| 6 | Keep open. Desired and observed status exists, but release, deployment generation, health, and compensation are incomplete. |
| 7 | Keep open. The local tree persists RUNNING before work, but the database does not yet make every acknowledged write durable and interruption recovery needs a full staging run. |
| 8 | Keep open. Concurrent audit compensation no longer restores over a newer app state. Runtime event ingestion, archive storage, and tamper evidence remain. |
| 9 | Keep open. Promotion now compensates a lease when Nomad submission fails. Rollback recovery and enforcing workload identity remain. |
| 10 | Keep open. Production identity, revocation, signed attestation, strict UI login, and operator sessions remain. |
| 11 | Keep open until a clean room user changes an export, reruns its contract, and reimports or shares the changed pack. |
| 12 | Keep open until stream and local profiles pass their own staging bars and the missing architecture decisions are recorded. |

## Recommended merge sequence

1. Freeze the historical stack. Record a commit disposition map and range diff
   from PR 1 through PR 22.
2. Build a platform contracts PR from the current integrated tree. Include only
   invariants that pass independently. Split unsafe or incomplete side effect
   lifecycles into open follow up work.
3. Build an owned packs PR with the 17 compact starters and their contracts.
4. Build an evaluation PR that reproduces the scorecard and clean room export
   checks without committing temporary browser or build output.
5. Build a clinician experience PR with the Practice Studio UI, the journey,
   transcript based tasks, and curated screenshots.
6. Run `scripts/merge-gate.sh --full` on every current head. Request human review
   after the automated evidence is green.
7. Merge one group at a time. Rebase and retarget the next group to `main`.
8. Close or supersede the old stacked PR only after the disposition map shows
   where every decision and fix landed.

## Immediate repair list

1. Apply `cargo fmt` to the post op scaffold and rerun all GitHub checks.
2. Keep stubs out of the production green state. Preserve only the isolated
   synthetic demo exception.
3. Add a recovering state for rollback after Nomad stops a job but Vault lease revocation fails.
4. Prove interruption recovery with Nomad, Vault, and Postgres configured.
6. Add a clean room change, contract, export, and reimport test for issue 11.
7. Run moderated sessions using `docs/research/anthony-doctor-jobs.md`.
