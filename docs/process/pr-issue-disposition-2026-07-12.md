# Pull request and issue disposition after live infrastructure testing

## Decision

None of the ten open pull requests should merge as they stand. None of the
eleven open issues meets every item in its current definition of done.

The integrated local tree proves enough work to narrow issues 2, 3, 5, 6, 7,
8, 9, and 11. The old stacked pull requests should be replaced by small pull
requests based on `main`. Their useful commits remain evidence, but the stack
is no longer the safest delivery path.

## Evidence used

The current local tree passed these checks:

| Proof | Result |
|---|---:|
| Rust platform tests | 90 passed |
| Simulated pressure test | 89 passed |
| Nomad, Vault, and Postgres pressure test | 124 passed |
| Workflow evaluation | 458 of 458 passed |
| Built artifact evaluation | 432 of 432 passed |
| Runnable app packs | 17 |

The live infrastructure run scheduled a generated Rust app on a Nomad client,
started its Docker container, published its allocated port, and received HTTP
200 from its health route. It also proved Vault template rendering, database
credential use and revocation, Postgres restart recovery, and Nomad rollback.

## Open pull requests

| PR | Live head result | Useful work | Decision |
|---:|---|---|---|
| 1 | GitHub checks pass. There is no review. | UI and first control plane contract. | Replace. It combines too many concerns and is the base of the whole stack. |
| 13 | GitHub checks pass. There is no review. | First staging, export, and routing seams. | Replace. Its staging proof did not execute an allocation and its external calls were made under the platform write lock. |
| 14 | GitHub checks pass. There is no review. | First runnable pack pattern. | Preserve the pack work in the packs replacement PR. Do not merge the stacked PR directly. |
| 15 | The post operation scaffold formatting check fails. There is no review. | Evidence gate types and adversarial fixtures. | Replace. The local tree fixes formatting and prevents stubs from authorizing patient data. |
| 16 | The inherited scaffold check fails. There is no review. | Postgres lifecycle schema and operation rows. | Replace. The live head does not durably write RUNNING before work. The local tree does. |
| 17 | The inherited scaffold check fails. There is no review. | Audit broker, HMAC, and sink probes. | Replace. The live head can restore a stale app over concurrent work. The local tree uses an exact state check. |
| 19 | The inherited scaffold check fails. There is no review. | Transit, database leases, policy mounts, and Vault audit. | Replace. The live head can leak a lease when Nomad submission fails. The local tree compensates that failure. |
| 20 | The inherited scaffold check fails. There is no review. | Tenant isolation, role checks, and principal bound report digest. | Replace. Static tokens and a report hash do not meet the production identity or signature bar. |
| 21 | The inherited scaffold check fails. There is no review. | Evaluation, refusal, and observed status work. | Split. Seven commits mix unrelated closeout work and depend on every earlier PR. |
| 22 | The inherited scaffold check fails. There is no review. | Journey evidence. | Preserve the evidence in the clinician experience replacement PR. Do not merge this stacked PR directly. |

Every open PR is reported as mergeable by Git. This only means Git can combine
the branches. PRs 15 through 22 still have a failing required check, and no PR
has a human review or approval.

## Open issues

| Issue | Confirmed now | Still missing | Decision |
|---:|---|---|---|
| 2 | One command starts the Docker infrastructure locally. Nomad scheduled a real allocation. Vault and Postgres checks passed. | The control plane still starts in a second command. The issue also asks for a Terraform staging variant and a blocking CI staging job that executes this path. | Keep open and narrow to one command for the whole stack, Terraform parity, and CI. |
| 3 | Broken encryption, audit, allowlist, and synthetic guards fail tests. Stubs cannot authorize patient data. | Egress is not observed from a network policy. Dependency scanning is not a real scanner for every export. Not every built in gate has artifact evidence. | Keep open and narrow to runtime egress and real dependency scanning. |
| 4 | Rules and HTTP model drivers share one ladder. Operation routing and failure are durable and tested. | There is no Claude driver that writes workspace diffs. Prompts exist for only one pack. Conversation state is not stored as the issue requires. Three model generated packs have not passed staging without hand edits. | Keep open unchanged. |
| 5 | All 17 packs compile, boot, run their owned job contract, and export source. All 17 include synthetic data. | Only one pack has prompts, policies, and gates folders. Only eight have docs folders. The registry signature does not cover each full folder archive. | Keep open and narrow to the full folder format and archive signature. |
| 6 | Nomad submission, a running allocation, HTTP health traffic, desired and observed status, and stop on rollback are proven. | Sandbox allocation isolation, preview routing, release separate from deploy, generations, and zero remaining allocation records are not proven. | Keep open and narrow to exposure, generations, and sandbox isolation. |
| 7 | Postgres transitions, operation rows, app recovery, audit recovery, and a signal 9 restart passed. RUNNING is written before driver work. | The test kills a live control plane after promotion, not during a promotion with an interrupted operation later marked failed. Some nonstage writes still allow degraded database durability. | Keep open and narrow to interrupted promotion recovery and write classification. |
| 8 | File and Postgres sinks, registration probes, HMAC fields, sink failure refusal, restart recovery, and concurrent compensation are tested. | Object archive retention, export digests, runtime app event ingestion, and independent tamper evidence are missing. | Keep open and narrow to archive integrity and runtime ingestion. |
| 9 | Transit round trip, rotation, tenant policy, Nomad Vault template rendering, one hour database credentials, credential authentication, and rollback revocation passed. | Nomad uses the development root token. Workload identity and policy limited allocation tokens are missing. Per tenant database roles remain shared. | Keep open and narrow to enforcing workload identity and tenant roles. |
| 10 | Two tenant isolation, role denial, actor attribution, strict token mode, and idle expiry pass. | OIDC, NPI upgrade, token revocation, an independently verifiable signature, and time limited operator sessions are missing. | Keep open unchanged. |
| 11 | Exports include source, tests, documentation, compliance evidence, Nomad, Render, Fly, Kamal, a derived pack, and customization instructions. The exported app builds and runs in evaluation. | A new person has not completed the issue's clean room handoff without platform help. Registry submission and sharing are not tested. | Keep open and narrow to the human handoff and sharing proof. |
| 12 | Seventeen packs and 78 scenarios are represented. Four out of scope scenarios are refused with reasons. A 12 app profile review exists. | Stream and local profiles do not meet their native runtime bars. The requested isolation, voice economics, and hospital boundary decisions are incomplete. | Keep open and narrow to profile native proofs and the three decisions. |

## Bugs found by executing the real allocation

The server only test could not reveal these bugs:

1. The pressure test expected production infrastructure for a stubbed app that
   was correctly limited to the synthetic demo pool.
2. Docker Desktop did not delegate a cgroup parent to the Nomad client.
3. The Nomad client registered in `dc1`, while jobs required `nyc3`.
4. Docker Desktop reported zero CPU capacity to Nomad, so placement failed.
5. The job used Docker Compose style `tmpfs`, which the Nomad Docker driver
   rejected.
6. The job did not declare `ports = ["http"]`, so Nomad reserved a port but
   Docker did not publish it.
7. The generated app bound to loopback inside its container, which would make
   a published port unreachable.
8. Nomad and the Docker daemon did not share one absolute allocation path, so
   Docker rejected the allocation mounts.
9. The production image points at a private placeholder registry, so local
   staging needed an explicit and recorded image override.
10. The pressure test did not distinguish a submitted job from a running and
    healthy allocation.

The local tree fixes all ten items and adds three required checks for a running
allocation, an HTTP health response, and the synthetic data guard.

## Work that can be eliminated

The following work no longer needs a separate implementation ticket:

* A second local staging harness for macOS. `scripts/staging-docker-up.sh`
  provides the path.
* A simulated Nomad allocation layer. The local proof now uses a running
  Nomad allocation.
* A separate fix for RUNNING write order. The integrated tree contains it.
* A separate fix for stale audit compensation. The integrated tree contains it.
* A separate fix for the promotion lease leak. The integrated tree contains it.
* A separate export manifest ticket. Nomad, Render, Fly, and Kamal files are
  already in every export.
* More synthetic starter packs as a count goal. All 17 exist and run. Future
  pack work should improve the common format and domain evidence.

The old stacked delivery path can also be eliminated after its commits are
mapped into replacement pull requests. Do not close the old PRs until that map
names the replacement commit for each useful change.

## Replacement pull requests

Create these branches from `main`:

1. Platform contracts. Include state, operation ordering, audit compensation,
   tenancy, and their focused tests.
2. Live infrastructure. Include Docker staging, Nomad job corrections, Vault
   compensation, the local image, and the 124 check proof.
3. Owned packs. Include all 17 runnable packs, synthetic datasets, and artifact
   contracts.
4. Export and evaluations. Include the export bundle, 78 scenario harness,
   scorecard, and clean room automation when it exists.
5. Clinician experience. Include the UI, journey, research tasks, and selected
   screenshots.

Each replacement PR must pass its own checks from `main`, stay within a stated
scope exception when needed, and receive human review before merge.
