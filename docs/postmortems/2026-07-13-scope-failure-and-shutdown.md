# HashiStack Healthcare Studio scope failure and shutdown

## Metadata

- Status: final
- Incident class: product delivery failure
- Declared: 2026-07-13
- Owner: repository owner
- Customer impact: no clinical users or patient-care workflows were launched
- Data impact: synthetic data only; no known PHI was processed
- Related work: PRs #32, #35, #38, #40, #42, #63–#67, #78, and #79

## Executive summary

HashiStack Healthcare Studio did not reach a minimum lovable product. The work
began as a clinician-facing builder and expanded into a local HashiStack,
DigitalOcean staging and production, deploy previews, authentication, import
and export, a local model tier, evidence tooling, and documentation comparable
to a mature infrastructure product. Each addition was defensible alone. In
combination, they replaced the user outcome with a platform program.

The repository produced useful components and credible point proofs. It did not
produce a complete, human-reviewed path from idea to a customizable and
exportable clinical application. The owner stopped the project when further
work would have increased cost and complexity without reducing that central
risk. The runtime was decommissioned and the repository was archived.

## Impact

No production clinical service was launched, so there was no direct patient or
care-delivery outage. The impact was lost time, cloud spend, attention spread
across too many workstreams, and an unfinished user promise. DigitalOcean had
$0.28 of identifiable droplet usage through the last available invoice preview;
final usage can lag. Netlify project sites and DigitalOcean runtime resources
were removed.

## What the project intended to prove

A doctor could start without signing in, describe a useful application, inspect
and customize the result, and export it. The system would teach through the
work, use a bounded local model tier, and run locally and on a simple cloud host.

## What was actually proved

- Rust services, browser flows, and synthetic fixtures could pass local checks.
- Exact commits could be deployed to a DigitalOcean host and Netlify previews.
- Import and re-export paths could fail closed when their source was missing.
- Gemma could be routed behind a bounded interface, but its latency exceeded a
  strict 90-second proof window in the final staging attempt.
- Documentation and evidence artifacts could describe isolated successes.

These were component proofs. They did not add up to the human-reviewed customer
journey required by issues #3, #10, and #11.

## Timeline

- 2026-07-11: the goal called for a clinician builder informed by HashiCorp
  architecture and documentation.
- 2026-07-12: local infrastructure, DigitalOcean deployment, production proof,
  and artifact profiling became parallel goals.
- 2026-07-13: Netlify production and staging, authentication, import recovery,
  export fidelity, Gemma routing, and minimum-lovable steering were active at
  the same time. PRs #78 and #79 remained open.
- 2026-07-13: the owner declared the scope too large and stopped the project.

## Thread of execution

The delivery path began with a simple UI. The team then added runtime services,
proof tooling, cloud infrastructure, deploy previews, identity, an agent/model
tier, import and export hardening, and a broader documentation system. Failures
in one layer usually generated another layer or proof requirement. Work moved
through many stacked PRs while the complete anonymous build-customize-export
journey remained unverified by a new human user.

The final staging run showed the pattern clearly: the exact application commit
could become healthy, but Gemma timed out at the proof boundary. The response
was another steering PR that changed runtime and deployment expectations. That
PR was technically green, yet accepting it would have expanded the system again
without settling whether the product solved the original job.

## Root causes

### 1. The product boundary was not held

Reference architectures became delivery requirements. Nomad, Vault, Packer,
DigitalOcean, Netlify, Clerk, Gemma, import/export fidelity, and extensive proof
all entered the critical path. The code-level analogue is *speculative
generality*: the system paid for future flexibility before one workflow had
earned it. The Code Complete challenge was managing complexity and requirements
change, not implementation effort.

Evidence: the open issue set still spanned identity, audit durability,
sandboxing, pack signing, provenance, workspace editing, deployment, and visual
polish when the project stopped.

### 2. Proof surfaces grew faster than the user journey

Local, staging, production, preview, screenshot, profiling, and documentation
proofs were built in parallel. This was *divergent change*: the repository had
too many independent reasons to change. The Code Complete challenge was system
integration. Component checks were green while the stranger test and the
mission-critical customer flow remained open.

Evidence: PRs #78 and #79 were mergeable with green checks, but issues #3, #10,
and #11 still had unresolved production safety and human acceptance criteria.

### 3. External latency and platform state were treated as late integration details

Gemma, Netlify, DigitalOcean, DNS, and GitHub deployment state introduced
variable timing and coordination. The code-level analogue is temporal coupling:
success depended on several systems completing in the assumed order and time.
The Code Complete challenge was defensive integration and performance design.

Evidence: the final exact commit deployed healthy, while the strict Gemma proof
failed at the 90-second boundary and the fallback correctly did not count as a
model-backed success.

## Contributing conditions

- Several PRs and worktrees were active at once.
- Architecture decisions and product acceptance were mixed in the same PRs.
- The success definition changed as new infrastructure became available.
- Documentation volume made isolated progress look closer to completion than
  the end-to-end experience was.

## Counter-signals that were easy to overweight

Green checks, healthy containers, polished screenshots, and exact-SHA deploys
were real. They were not evidence that a clinician could finish the intended
job. The team needed one outcome scorecard above every component scorecard.

## Stochastic lens

Model response time, cloud provisioning time, DNS propagation, concurrent Git
changes, and third-party platform behavior were variable. The architecture
treated several of them as synchronous prerequisites. A narrower product would
have mocked external systems in CI, used durable jobs for slow generation, and
required only one provider and one environment for initial proof.

## Inversion exercise

To repeat this failure, accept a new platform for every local failure, keep
several PR stacks active, let documentation substitute for human acceptance,
add staging and production before the local customer flow is complete, and omit
time, cost, and kill limits. The successor project adopts the opposite rules.

## Corrective actions for any successor

1. Approve data sourcing, data model, data flow, and network topology before
   application code or cloud deployment.
2. Hold the first release to two artifacts: one reviewed staff handoff document
   and one reviewed patient-education slide deck.
3. Use synthetic or deidentified inputs until privacy, security, vendor, and
   clinical review explicitly permit patient data.
4. Require every clinical claim to cite an approved source. Social-media data
   may indicate communication context; it may never establish clinical truth.
5. Keep one repository, one active product PR, one generation provider, and one
   local runtime until both workflows pass a human acceptance test.
6. Use durable job state for external generation and deterministic gates for
   provenance, PHI, format, and approval.
7. Stop or rescope if either workflow cannot be demonstrated locally within the
   agreed time and cost budget.

## Evidence provenance

The Git history, closed PRs, closed issues, Actions runs, and committed
evaluation records are retained in this archived repository. Runtime resources
and deploy sites were intentionally deleted. The shutdown inventory is recorded
in `docs/operations/2026-07-13-shutdown-record.md`.
