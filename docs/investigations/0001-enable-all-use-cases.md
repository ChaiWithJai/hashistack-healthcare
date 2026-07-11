# Investigation 0001 — Enabling all seeded use cases across the HashiStack

Tracking issue: [#12](https://github.com/ChaiWithJai/hashistack-healthcare/issues/12)
Goal + bar: [docs/GOAL.md](../GOAL.md) · Design authority: [RFC 0001](../rfc/0001-clinician-platform.md)

## The question

Can a doctor or CHP vibe-code **every** use case we seeded — 21 validated, 17
in platform scope, 4 refused with reasons — to the GOAL.md bar, on this
stack? For each: what must be true of the platform, what's the cheapest spike
that proves or kills it, and which architectural trade-offs does it force?

## Method

Investigate by **profile, not by use case**. The RFC's central simplification
is that 17 use cases compile to 3 infrastructure shapes; if a profile works
at the bar for one use case, the remaining work per use case is a pack
(manifest + scaffold + prompts + gates + synthetic data), not platform work.
So: three profile spikes, then seventeen enablement cards.

Every spike lands in the staging environment (#2) and extends the pressure
test — a spike that can only be verified by manual smoke testing is not done.

### Spike 1 — `web` profile (10 use cases; Waves 1–2)

The default path, already contract-proven in simulation. The spike is the
ticket chain: staging (#2) → evidence gates (#3) → Claude driver (#4) →
runnable packs (#5) → real allocations (#6) → durable state/audit (#7, #8) →
Vault (#9) → identity (#10) → eject (#11).

**Exit test:** all three Wave 1 packs describable → gated → promoted →
ejected at the bar, in staging, by someone who isn't us.

### Spike 2 — `stream` profile (rpm-wearables 3, visit-notes 8, ambient-scribe 11; Wave 3)

New platform surface: pinned min-instances (Nomad `service` jobs with
canaries disabled on drain), WebSocket/SSE through the ingress router, a
queue, and **voice drivers** as the first non-model driver family.

- Decision to record: **Retell (bundled BAA, rented margin) vs self-hosted
  LiveKit on the prod pool (owned margin, owned ops)** — RFC open question 3.
  Run both behind one `VoiceDriver` trait in staging; measure ops-hours and
  per-session cost, then decide on data.
- Gate delta: streams need a *session* audit shape (who heard what, when)
  rather than request-shaped audit events — feeds #8's design.

**Exit test:** a synthetic "visit" streamed end-to-end with the transcript
landing in the audit stream and the allocation surviving a node drain.

### Spike 3 — `local` profile (deid-local 17, note-extraction-local 18, airgapped 19, hybrid 20; Wave 4)

The inversion: no server allocation at all. The pack ships a quantized
on-device model runtime; the platform's role shrinks to **signing, gates,
docs, and the optional non-PHI web sidecar**.

- The gate engine must therefore evaluate *artifacts* (the pack bundle),
  not running allocations — this is why #3's validate/execute split matters
  now, not in Wave 4.
- Decision to record: what the platform attests for code it doesn't host
  (signature chain + reproducible build + local audit log format).

**Exit test:** deid-local pack installed on a laptop with networking
disabled; its local audit log imports cleanly into the platform's export
format.

### The refusals (9, 10, 15, 21)

Not a spike — a product surface. The describe endpoint must recognize these
shapes and refuse with the RFC's written reasons. Add refusal fixtures to the
pressure test: `"triage bot for chest pain"` → 422 with a reason, never a
scaffold.

## Deliverables

1. **Enablement matrix** — 17 rows × (profile, required gates, required
   drivers, synthetic-data source, blockers, ticket refs, status). Lives in
   this file, updated as spikes land.
2. **Decision records** (docs/decisions/) for: isolation upgrade
   (Firecracker vs gVisor, by isolation-per-ops-hour — RFC OQ1), voice
   economics (OQ3), hospital tenant boundary (namespace vs dedicated pool —
   OQ4), and pack signing chain (platform key vs clinician identity — OQ2).
3. **Re-scored wave plan** once the matrix has data.

## Prior art to reference for every architectural decision

**In-house, already distilled** — [docs/hashicorp-steering.md](../hashicorp-steering.md),
read directly from the five source trees:

| Source | Pattern to reach for | Where it bites here |
|---|---|---|
| Nomad `plugins/drivers` | required trait + optional capability traits, self-described config schemas | agent/voice/deploy driver families (#4, spike 2) |
| Nomad `structs` | desired vs observed status, promotion as first-class state | real allocations (#6) |
| Vault `sdk/logical` + `audit/` | uniform request envelope; **no audit write, no operation**; salted-HMAC fields | gate engine + audit pipeline (#3, #8) |
| Packer SDK | prepare/run split, version-pinned plugin requirements | gate dry-runs, pack pinning (#3, #5) |
| Waypoint | operations upserted before work; release ≠ deploy; generations; URL service | lifecycle rows (#7), promotion (#6), preview URLs |
| Boundary | DB-enforced state tables; Target/Session operator access; ct/pt envelope fields | control DB (#7), operator access (#10), PHI storage |

**External, per decision:**

- *Agent loop & scaffold-first generation:* Lovable teardown (GitHub sync as
  the trust feature; describe→deploy gap), v0/Replit Agent postmortems,
  Claude Agent SDK. The gate is our differentiation — keep it in every demo.
- *Sandbox isolation:* Firecracker (E2B, Fly Machines as production
  microVM-per-untrusted-workload prior art), gVisor (GKE Sandbox), and
  `nomad-driver-firecracker`/`containerd` maturity. Evaluate by
  isolation-per-ops-hour, not benchmark vanity.
- *Healthcare substrate:* Medplum (open-source HIPAA app platform — their
  middleware/audit shapes), Synthea (synthetic patients for `synthetic/`),
  SMART on FHIR (identity + scopes prior art for #10), Aptible (BAA-first
  PaaS positioning and its limits — why we own the scheduler).
- *Compliance-as-code:* OPA/Sentinel policy models (gates as policy), OSCAL
  (machine-readable control catalogs — map gate ids to HIPAA safeguard
  citations in the eject docs), cargo-audit/osv-scanner (dependency gates).
- *Portability/eject:* Kamal + DHH's colo math (RFC Phase 3 checkpoint),
  Rails app templates and `cargo generate` (template ejection UX), Render
  blueprints / fly.toml (export targets, #11).
- *Clinical guardrails:* Ong/Antaki three-tier framework (the wall we're
  bridging — every enablement card cites its tier), Grover's "threshold for
  building responsibly" (the gate's design brief).

## Trade-off rubric

Every decision record scores its options against, in order:

1. **Does the doctor's workflow change?** If yes, it's probably wrong
   (Tao 1). Technology swaps must be invisible above the platform layer.
2. **Is the compliance story evaluable?** A control we can't gate on in CI
   (#3) is a promise, not a control.
3. **Isolation-per-ops-hour** for anything touching the sandbox boundary.
4. **Margin vs ops ownership** for anything rented (voice, model inference).
5. **Eject cleanliness** — does it survive leaving the platform? (#11)

## Status

| Use case (RFC #) | Pack | Profile | Status |
|---|---|---|---|
| 1 hypertension-tracker · 4 compliance-checklist · 6 patient-intake | shipped (manifest) | web | contract-proven in simulation; real enablement = #2–#11 chain |
| 2 post-op-monitor · 14 insurance-verification | shipped (manifest) | web | same, storyboard packs |
| 5, 7, 12, 13, 16 | wave 2 backlog | web | blocked on spike 1 exit |
| 3, 8, 11 | wave 3 backlog | stream | blocked on spike 2 |
| 17, 18, 19, 20 | wave 4 backlog | local | blocked on spike 3 |
| 9, 10, 15, 21 | — | refused | needs refusal surface + pressure-test fixtures |
