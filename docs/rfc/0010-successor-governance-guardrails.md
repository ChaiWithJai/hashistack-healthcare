# RFC 0010: Guardrails for a successor clinical communications service

- Status: accepted for discovery
- Date: 2026-07-13
- Related postmortem: `docs/postmortems/2026-07-13-scope-failure-and-shutdown.md`

## Decision

Any successor begins as a local, docs-first Rust API with two outputs only:

1. a reviewed Gamma document that teaches staff through a clinical handoff;
2. a reviewed Gamma slide deck that helps a patient understand one conversation.

No patient data, cloud runtime, multi-agent tier, general application builder,
or new generation provider enters scope before both synthetic flows pass their
acceptance tests.

## Required architecture gate

The owner must approve four records before implementation: data sourcing, data
model, data flow, and network topology. Each record names an owner, unresolved
risk, and rejection condition.

## Clinical and instructional boundary

Authoritative clinical sources establish truth. TikTok or other social content
can reveal language, misconceptions, and topics, but cannot support a clinical
claim. A clinician approves every exported artifact. For instructional design,
the trainer owns Gagné events 1–4 and the system amplifies events 5–9.

## Delivery boundary

Start with one Rust API, Postgres-backed durable jobs, provider ports, outbound
allowlists, and deterministic egress checks. Gamma is an artifact-generation
provider, not a source of truth. External calls are mocked in CI.

## [PM COMMENT]

This decision prevents the failed project’s main recurrence: treating platform
capability as product scope. It converts the postmortem’s corrective actions
into an entry gate that must be satisfied before code or cloud spend.

## [PRE-MORTEM]

Assume the successor fails in six months. The likely causes are unlicensed
social data, untraceable clinical claims, PHI sent to a vendor, a renderer used
as a clinical authority, or another infrastructure expansion before human proof.
The controls are authorized ingestion only, claim-level citations, default-deny
PHI egress, provider separation, one active PR, and explicit stop conditions.

## Rejected alternatives

- Reusing the full HashiStack runtime: too much operational surface for two
  document workflows.
- Building a general clinical application platform: repeats the failed scope.
- Scraping TikTok: creates legal, reliability, and provenance risks.
- Sending output without clinical review: unacceptable for patient-facing work.
