# Proof for the stacked pull requests

## Purpose

This report supports a trunk based merge sequence. Review one pull request,
squash it, merge it into `main`, retarget the next pull request, and rerun its
proof. Stop when a person must decide a product, security, or clinical tradeoff.

The report does not claim that every live PR head passes. It records what each
PR introduced, what the integrated local tree proves, what changed after the PR
head, and what a reviewer must decide.

## Whole stack profile

| Measure | Result |
|---|---:|
| Rust platform tests | 90 passed |
| Simulated pressure checks | 89 passed |
| Nomad, Vault, and Postgres checks | 130 passed |
| User workflow checks | 458 of 458 passed |
| Built artifact checks | 432 of 432 passed |
| Scenarios | 78 |
| Packs | 17 |

The live infrastructure profile includes a Nomad client allocation. Nomad
started the generated Rust app, published its HTTP port, and received a 200
response from `/health`. Vault rendered a database credential into the task.
Rollback stopped the job and removed the Postgres role.

## Visual before and after

The first image shows the generated app before release. It is a sandbox and
uses synthetic data. The second image shows the released state after the gate
and co-sign path.

| Before release | After release |
|---|---|
| ![Sandbox before release](../evals/journey/01-sandbox.png) | ![Released app](../evals/journey/03-live.png) |

The first artifact image shows the exported app at its starting screen. The
second image shows the app after a user completes its owned clinical task.

| Exported starting state | Exported task result |
|---|---|
| ![Exported app home](../evals/journey/04-artifact-home.png) | ![Exported app result](../evals/journey/06-artifact-flag.png) |

The full journey is in [journey.md](../evals/journey/journey.md). Screenshots
for all 17 packs are under `docs/evals/screenshots/`.

## Pull request proof

| PR | Before | After in the integrated tree | Proof | Human review |
|---:|---|---|---|---|
| 1 | Static wireframes and simulated state. | The same clinician flow now reaches tested services and owned exports. | UI screenshot, health contract, and workflow tests. | Confirm that the product language does not imply production readiness. |
| 13 | No local HashiStack lifecycle and an export promise. | Docker staging runs Nomad, Vault, Postgres, and the control plane. Every export contains source and ownership docs. | 130 infrastructure checks and export contract tests. | Review the staging trust boundary and export ownership claims. |
| 14 | A pack was a manifest with feature strings. | The post operation pack builds and runs as an exported Rust app. | Standalone Cargo tests and the exported app screenshots. | Review whether the clinical workflow is useful and safely limited. |
| 15 | Gates trusted controls reported by the app. | Broken source fails evidence checks. A stub cannot authorize patient data. | Evidence contract tests and the failing gate screenshot. | Review which controls are strong enough for patient data. |
| 16 | A restart lost app state and operations. | Postgres restores apps, operations, allocation handles, attestations, and audit events. RUNNING is stored before driver work. | Signal 9 restart section of the 130 check run. | Review which writes must fail when Postgres is unavailable. |
| 17 | Audit lived in memory and a sink failure could not block work. | File and Postgres sinks confirm writes. Sensitive values use HMAC. Audit compensation cannot erase a newer app change. | Audit broker tests, sink failure test, and restart proof. | Review retention, archive integrity, and access to plaintext views. |
| 19 | Vault existed only in labels and rendered text. | Transit rotation, template rendering, database login, and revocation execute locally. Promotion compensates a lease when Nomad submission fails. | Vault sections of the 130 check run. | Review the remaining root token and shared database role. |
| 20 | API actions used one hardcoded doctor and tenant. | Two tenants, role checks, strict token mode, and idle expiry pass. The report digest binds to the authenticated principal. | Ten identity contract tests and pressure test tenant checks. | Decide the OIDC provider, signature service, and operator access policy. |
| 21 | Each feature had separate evidence and status language. | One harness profiles the workflow, refusals, identity, and observed deployment status. | 458 workflow checks and 432 artifact checks. | Decide whether this large mixed change should be split before squash. |
| 22 | The stack had many test results but no single audience journey. | The journey records sandbox, gate, release, export, task completion, and audit events. | Six journey screenshots plus the profile JSON. | Review whether the journey represents the intended design partner. |

## Bugs found after the live PR heads

The live allocation test found problems that do not appear in the current PR
checks:

* The pressure test mixed a synthetic demo with production infrastructure.
* The Nomad job used an invalid tmpfs field.
* The Docker task did not publish its reserved HTTP port.
* The generated app bound to loopback inside the container.
* The Nomad client used the wrong datacenter and reported no CPU capacity.
* Docker and Nomad did not share one allocation path.
* The live PR chain did not require a running allocation or health traffic.

The integrated local tree fixes these problems. These fixes must enter the
chain before PR 13 can be considered proven.

## Why this is the smallest solution that meets the goal

The product goal requires one complete learning loop. A doctor describes an
app, sees a synthetic preview, understands why release is blocked, fixes the
supported controls, releases or publishes a synthetic demo, and exports code
that another person can change.

The current design uses one Rust control plane, one pack format, one gate
engine, one operation record, and one export format. Nomad, Vault, and Postgres
are used only where the goal needs scheduling, credentials, and durable state.
The deterministic rules driver remains the default, so local tests do not need
a paid model or patient text.

Adding more infrastructure would not close the remaining human decisions.
Those decisions concern clinical usefulness, production identity, signed
attestations, workload identity, and audit retention. The merge sequence should
therefore preserve the smallest tested implementation and stop for review at
those boundaries.

## Squash order

Use this order:

1. PR 1 establishes the product and API contract.
2. PR 13 adds the service seams, but it must include the later allocation fixes.
3. PR 14 adds the first owned runnable pack.
4. PR 15 makes the gate evidence based.
5. PR 16 adds durable state and operation ordering.
6. PR 17 makes audit load bearing.
7. PR 19 adds Vault lifecycle proof.
8. PR 20 adds tenant and role enforcement.
9. PR 21 adds the combined evaluation and observed status surface.
10. PR 22 adds the audience journey.

After each squash, retarget the next branch to `main` and rerun its focused
proof. Do not carry an inherited red check forward.
