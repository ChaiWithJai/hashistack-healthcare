# RFC 0001 — Platform design: Lovable for clinicians on HashiStack + DigitalOcean

Status: accepted. The model policy was amended by
[decision 0009](../decisions/0009-agent-workspace-and-model-routing.md).
Design authority: the Tao of HashiCorp, applied literally, with Mitchell Hashimoto as the organizing lens for every decision.
Wireframes: [docs/design/](../design/) (storyboards 1a/1b/1c + architecture 1d)
Steering: [docs/hashicorp-steering.md](../hashicorp-steering.md) (patterns read out of the nomad/vault/packer/waypoint/boundary trees)

## Summary

A clinician-facing app builder where doctors describe tools in natural language and receive running, HIPAA-scaffolded applications. Two layers: an infrastructure layer (DigitalOcean under BAA, orchestrated by Nomad/Vault/Terraform/Packer) and an application layer (agent loop, use case packs, compliance runtime). All 21 validated use cases ship as declarative packs on three runtime profiles. Extension happens through plugins and packs, never through forks of the core.

## Motivation

Clinicians are already building (Saleem, Hobbs, Qiu, Song, Brophy, Grover, Blashki). Every consumer platform they use fails at the same two points: no BAA, no path from prototype to compliant deployment. The Ong/Antaki three-tier framework names the wall precisely: Tier 1 and 2 tools work on consumer platforms, Tier 3 (patient-facing, PHI-handling) stalls. This platform is the bridge, built compliant-first instead of compliance-retrofitted.

## Design principles (the Tao, applied)

1. **Workflows, not technologies.** The doctor's workflow is fixed: describe, preview, iterate, deploy, audit. Every technology below it is swappable. If Firecracker gets replaced by something better, the doctor never notices.
2. **Simple, modular, composable.** Each component does one thing. The agent does not deploy. The deployer does not audit. The auditor does not schedule. Unix philosophy at service granularity.
3. **Immutability.** Sandbox images are Packer-built, versioned, never mutated in place. A doctor's app redeploys as a new allocation, never patches a running one. This is also a compliance argument: immutable images mean auditable provenance.
4. **Codification.** Everything is declarative files in git: infrastructure (Terraform), images (Packer), jobs (Nomad HCL), policies (Vault/Sentinel-style), use case packs (the pack spec below). Nothing is click-ops. The audit trail is the git log plus Vault audit log.
5. **Automation through APIs.** Every internal service exposes an API first, UI second. The doctor UI is one client of the control plane. A future CLI, a hospital's integration, a community tool are other clients. No privileged UI.
6. **Pragmatism.** Ship the docker driver before the Firecracker driver. Ship one region before two. Ship three packs before twenty one.

The Mitchell signature move governing extensibility: **a small, stable core with a plugin protocol at every variation point.** Terraform has providers, Vault has secret engines, Nomad has task drivers, Packer has builders. This platform has packs, drivers, and gates (defined below).

## The workflow (fixed contract)

```
describe -> generate -> preview -> iterate -> gate -> deploy -> operate -> audit
```

- describe: natural language plus pack selection ("post-op recovery tracker for my practice")
- generate: agent loop produces code inside the pack's scaffold, hipaa-core pre-wired
- preview: app runs in an isolated sandbox at a per-session URL, synthetic data only
- iterate: conversational edits, checkpointed, rollbackable
- gate: automated compliance checks (the gates plugin point) before anything can touch real data
- deploy: promotion to a production allocation under the BAA boundary
- operate: logs, metrics, uptime, all visible to the doctor
- audit: every PHI access, every deploy, every config change, queryable

The gate step is the product. Lovable goes describe to deploy with nothing in between. The gate is where Grover's "threshold for building responsibly" becomes code instead of discipline.

## Layered architecture

```
+--------------------------------------------------------------+
|  APPLICATION LAYER                                            |
|  doctor UI (web) | agent loop | pack registry | gate engine   |
|  control plane API (everything above is a client of this)     |
+--------------------------------------------------------------+
|  PLATFORM LAYER                                               |
|  Nomad (scheduling) | Vault (secrets/keys) | ingress router   |
|  audit pipeline | Postgres control DB | object storage        |
+--------------------------------------------------------------+
|  INFRASTRUCTURE LAYER (DigitalOcean, under BAA)                |
|  Terraform-managed: VPC, Droplets, managed Postgres,          |
|  Spaces, Load Balancers | Packer-built images                 |
+--------------------------------------------------------------+
```

## Infrastructure layer on DigitalOcean

Scope note: DO signs BAAs on covered products only (Droplets, Kubernetes, Spaces, Load Balancers, managed databases per current list). Everything that can touch PHI stays inside covered products. Verify the current covered list before contract.

### Topology

- **One VPC per environment** (prod, staging). All Droplets private, single ingress path through DO Load Balancer to the router.
- **Control plane pool:** 3 small Droplets running Nomad servers + Vault (Raft storage). Odd count for quorum. These never run tenant workloads.
- **Sandbox pool:** Nomad clients tagged `role=sandbox`. Runs untrusted generated code during preview. No route to production databases. Synthetic data only. This network boundary is itself a compliance control.
- **Production pool:** Nomad clients tagged `role=prod`. Runs gated, promoted apps. Only pool with access to tenant Postgres databases.
- **Data:** DO Managed Postgres (covered product) with one logical database per tenant app, mirroring the monorepo local dev pattern. DO Spaces for file/image uploads (post-op photos etc), server-side encryption plus field-level envelope encryption from hipaa-core.

### Tooling

- **Terraform** owns everything: VPC, Droplets, LBs, Postgres clusters, Spaces, firewall rules, DNS. One repo, one state per environment, plan/apply in CI. → [terraform/prod/](../../terraform/prod/)
- **Packer** builds two image families: `control-plane` (Nomad server, Vault) and `client` (Nomad client, task drivers, hipaa-core runtime deps). Images are versioned and immutable; a security patch is a new image and a rolling replace, never an ssh session. → [packer/](../../packer/)
- **Nomad** schedules three job classes:
  - `system` jobs: router, audit shipper, on every client
  - `sandbox` jobs: preview allocations, docker driver at launch, Firecracker or gVisor driver as the isolation upgrade (this is a task driver swap, invisible above the platform layer, which is the whole point of principle 1)
  - `service` jobs: promoted production apps → [nomad/templates/](../../nomad/templates/)
- **Vault** is the compliance spine:
  - per-tenant transit keys backing hipaa-core encryptField/decryptField (keys never leave Vault) → [vault/policies/](../../vault/policies/)
  - dynamic Postgres credentials per allocation, short TTL, auto-revoked
  - LLM API keys scoped per environment
  - the Vault audit log is a HIPAA technical safeguard artifact, exportable for any security review
- **Consul and Boundary are deferred.** Nomad's native service discovery covers routing at launch scale. Boundary enters when hospital tenants demand session-recorded operator access. Deferred is a decision, not an omission (principle 6).

### Why DO and not Render/Railway for this layer

The platform needs scheduler-level and eventually hypervisor-level control to run untrusted multi-tenant generated code. A PaaS sells you the inside of a container; this design requires owning what wraps the container. DO Droplets under BAA are the cheapest metal that permits it. Render stays in the picture as a deployment *target* for doctors who want their promoted app exported (see portability, below), and the monorepo from earlier remains the pattern for one-off hosted apps outside the platform.

## Application layer

### Control plane services (each independently deployable, API-first)

1. **Planning service.** A private DigitalOcean Gemma 4 endpoint proposes two or three treatments from a bounded request. Rust checks the response and creates source from pack rules. Gemma has no tools, file access, deployment rights, secrets, or patient data. Prompts are versioned artifacts in git and tested like code. → `src/workspace_agent.rs`
2. **Pack registry.** Serves use case packs (spec below). Signed packs only. Community packs install from the public registry after signature verification. This is the "one signed core plus declarative packs" pattern from the Airplane Mode ADRs, promoted to platform scale. → `src/packs.rs`
3. **Gate engine.** Runs the promotion checklist as code: no PHI in prompts during preview, hipaa-core middleware present on every route the static analyzer flags as data-touching, auto-logoff wired, encryption key requested from Vault, dependency scan clean, third-party call allowlist satisfied (no un-BAA'd AI endpoints). Gates are plugins; hospitals can add their own (an IRB gate, a model-risk gate). → `src/gates.rs`
4. **Deploy service.** Renders the winning pack profile into a Nomad job, requests Vault policies, registers the route, records the deploy event in the audit stream. → `src/deploy.rs`
5. **Audit pipeline.** Every hipaa-core audit event, gate result, deploy, and Vault access flows to an append-only store. The doctor-facing "who touched what" view and the hospital-facing export both read from here. → `src/audit.rs`

### Runtime profiles (three, not twenty one)

Every use case compiles to one of three infrastructure shapes. This is the simplification that makes 21 use cases tractable.

| Profile | Shape | Nomad job class | Serves use cases |
|---|---|---|---|
| **web** | request/response container, Postgres, optional cron | service | 1, 2, 4, 5, 6, 7, 12, 13, 14, 16 |
| **stream** | persistent process, WebSocket/SSE, queue | service + min instances pinned | 3, 8, 11 (voice pipeline via Retell/LiveKit driver) |
| **local** | no server allocation; deterministic local processing, with an optional web sidecar for work that does not use patient data | none, or web sidecar | 17, 18, 19, 20 |

Out of platform scope by prior analysis: 9 (enterprise outcomes platform), 10 (ONC interoperability), 15 (triage liability), 21 (FDA device). The platform should refuse to scaffold these and say why; a refusal with a reason is a trust feature.

### Use case packs (the extension unit)

A pack is a declarative folder, same philosophy as a Terraform module or a Nomad job spec:

```
packs/hypertension-tracker/
  pack.hcl          # metadata, profile=web, tier=2->3, required gates
  scaffold/         # app template with hipaa-core pre-wired (the monorepo template, formalized)
  prompts/          # agent system prompts tuned for this clinical domain
  policies/         # Vault policy fragments, network allowlist
  gates/            # pack-specific checks (e.g. escalation path must exist before promotion)
  synthetic/        # synthetic dataset for preview mode
  docs/             # clinician-facing description, evidence citations
```

The 21 use cases become 17 packs (excluding the four refusals) shipped in waves:

- **Wave 1 (launch):** compliance-checklist (4), hypertension-tracker (1), patient-intake (6). Lowest risk, validated demand, all web profile.
- **Wave 2:** post-op-monitor (2), patient-portal (7), insurance-verification (14), nemt-logistics (16), clinical-dashboard (5).
- **Wave 3 (stream profile lands):** outbound-followup (13), inbound-scheduling (12), rpm-wearables (3), visit-notes (8), ambient-scribe (11) with voice vendor drivers.
- **Wave 4 (local profile lands):** deid-local (17), note-extraction-local (18), hybrid-pipeline (20), airgapped-support (19). These packs use deterministic local code. They do not add another model runtime.

## Open source and extension model

- **Open core:** the pack spec, the gate plugin interface, the agent driver interface, hipaa-core, and reference packs are open source. The hosted control plane, the pack signing service, and the BAA-covered infrastructure are the commercial product. (License note: HashiCorp itself moved MPL to BSL in 2023 and was acquired by IBM; pick MPL-2.0 or Apache-2.0 for the open pieces and decide the core license deliberately, it is a strategic choice not a default.)
- **Three plugin points, mirroring HashiCorp's pattern:**
  - **packs** (like Terraform modules): clinicians and communities publish domain packs; signing plus gate review before registry listing
  - **drivers** (like Nomad task drivers): voice drivers (Retell, LiveKit) and deploy target drivers (Nomad native, export-to-Render, export-to-Kamal). The planner is Gemma only and is not a plugin point.
  - **gates** (like Vault plugins): compliance checks as code; a hospital's own gates run alongside platform gates
- **Portability as principle:** every app exports as the monorepo shape (Dockerfile plus render.yaml/fly.toml/kamal deploy.yml). No hostage code. The Lovable teardown showed GitHub sync is a trust feature that drives adoption; here it is also the tier 2 to tier 3 bridge in reverse, letting a hospital take a validated app in-house.

## Rollout

- **Phase 0 (rented substrate, 6-8 weeks):** agent + pack registry + gate engine on Fly Machines or a single DO droplet, docker isolation, Wave 1 packs, 5-10 design partner clinicians from the documented builder communities (Offcall, Amplify Care, DrVibe cohorts). ← **this repo is the Phase 0 slice; see implementation status below**
- **Phase 1 (owned substrate):** Terraform/Packer/Nomad/Vault on DO under BAA, sandbox/prod pool split, promotion gates live, Wave 2 packs. First paying tenants.
- **Phase 2:** stream profile, voice drivers, Wave 3. Hospital pilot with custom gates.
- **Phase 3:** local profile, Wave 4, community pack registry opens. Basecamp math review: at sustained scale, run the DHH numbers on colo versus DO.

## Alternatives considered

- **Kubernetes instead of Nomad:** rejected on the DHH/Kamal precedent and the Tao. K8s is a technology commitment that dominates the workflow; Nomad is a scheduler that serves it. Also, Nomad task drivers are the cleanest path to microVM isolation without rearchitecting.
- **Fly.io as permanent substrate:** viable, and it is what Lovable chose. Rejected as the end state because the sandbox layer is the moat and the margin; renting it caps both. Kept as Phase 0 substrate.
- **Pure PaaS (Render/Railway):** cannot express the sandbox pool at all. Retained as export targets.

## Open questions

1. Firecracker driver maturity on Nomad versus gVisor via docker runtime class: benchmark both in Phase 0, decide by isolation-per-ops-hour.
2. Pack signing chain: platform key only, or clinician identity in the chain (an MD-verified badge is a trust product in itself)?
3. Voice profile economics: Retell bundles the BAA but owns the margin; LiveKit self-hosted on the prod pool keeps margin but adds ops. Decide per Wave 3 data.
4. Where does the hospital tenant boundary sit: namespace per hospital in shared Nomad, or dedicated pool? Dedicated pool is the safer default for the first hospital deal.

---

## Appendix A — Amendments from the HashiCorp source review

We read the nomad, vault, packer, waypoint, and boundary trees (commit history + extension-point interfaces); the full pattern write-up is in [docs/hashicorp-steering.md](../hashicorp-steering.md). Five amendments to the plan follow from it:

1. **Gate/driver plugin contracts get a Nomad-style split**: a small mandatory trait plus optional capability traits with explicit "not supported" defaults, and each plugin self-describes its config schema (Nomad `hclspec` pattern) so the control plane validates `pack.hcl` centrally while schemas live with plugins. The SDK ships as its own crate (Packer's SDK split), so pack authors never depend on the control plane.
2. **The audit pipeline adopts Vault's broker invariant: no audit write, no operation.** A deploy or PHI access that cannot durably land in at least one append-only sink returns an error instead of proceeding. Sensitive values in audit events are salted-HMAC'd with an explicit plaintext allowlist, Vault-style: correlatable, not disclosable.
3. **The app lifecycle becomes a database-enforced state machine** (Boundary's `session_valid_state` table): an `app_valid_state(prior, current)` table plus an append-only state-history table, so the describe→gate→deploy transitions are provable to an auditor even against application bugs. Statuses carry the Nomad dual axis: desired vs observed, with the gate as a first-class `requires promotion` state (Nomad canary promotion, Waypoint release ≠ deploy).
4. **Every workflow step becomes a Waypoint-style operation row**, upserted as RUNNING before work starts — crash-visible, resumable, auditable by construction. Generations (Waypoint) keep preview iterations from accreting allocations.
5. **Operator access (Phase 2+, hospitals) follows Boundary's Target/Session model** even before Boundary itself lands: time-boxed sessions minted against a policy object with max duration, connection limit, recording flag, and a `termination_reason` — never standing access. Boundary itself stays deferred; the shape arrives first.

Commit discipline adopted from the same review: scope-prefixed subjects (`gates:`, `packs:`, `deploy:`, `ui:`), behavior changes phrased as user-visible facts, `.changelog/` entries per PR once CI grows a release step.

## Appendix B — Phase 0 implementation status (this repo)

| RFC component | Status | Where |
|---|---|---|
| Control plane API (API-first, no privileged UI) | ✅ running | `src/api.rs` |
| Pack registry (signed packs only, HCL manifests) | ✅ 5 packs: Wave 1 + 2 storyboard packs | `src/packs.rs`, `packs/` |
| Gate engine (gates as plugins, preflight, false-pass guarded) | ✅ 9 built-in gates | `src/gates.rs` |
| Planning service (Gemma only; deterministic local fallback) | ✅ bounded planning is implemented | `src/workspace_agent.rs` |
| Deploy service (promote on green + co-sign, Nomad job render, rollback) | ✅ simulated allocations | `src/deploy.rs` |
| Audit pipeline (append-only, JSONL export) | ✅ in-memory | `src/audit.rs` |
| Doctor UI (wireframes 1a/1b/1c/1d as one wired client) | ✅ served at `/` | `web/index.html` |
| Terraform (VPC, pools, managed PG, prod-only db firewall, LB) | ✅ codified, not yet applied | `terraform/prod/` |
| Packer (control-plane + client image families) | ✅ codified | `packer/` |
| Vault (per-tenant transit + dynamic db creds policy) | ✅ codified | `vault/policies/` |
| Nomad (service job template the deploy service renders) | ✅ codified | `nomad/templates/` |
| Postgres control DB and real sandbox allocations | ⏳ next | — |
