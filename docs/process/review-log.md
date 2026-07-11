# Review log — dispositions and provisional decisions

Ambient record of interactive reviews: what was reviewed, what was decided
in the operator's stead (per the profile in CLAUDE.md), and what was flagged
for veto. A provisional decision stands unless vetoed; a veto here is cheap
and expected.

## Round 1 — 2026-07-11, six commits across two branches

Mode: commit-by-commit; from stop 2 onward, dispositions recorded ambiently
instead of polling the operator (their instruction at stop 1).

### Stop 1 — `demo:` (skinned UI over simulated control plane)

Reviewed live with the operator. Provisional decisions made from the
operator profile:

- **P1. Gate vocabulary is dual-register.** UI keeps clinical plain language
  ("encryption on all patient fields"); the ejected COMPLIANCE.md and the
  hospital-facing export additionally map each gate id to its HIPAA citation
  (e.g. auto-logoff → §164.312(a)(2)(iii)) via a static table in gates.rs.
  Rationale: best-in-class experience for the doctor, citation language for
  the security review — same report, two renderings. → fold into #3.
- **P2. Skins: 1a builder is the product default; 1c chart's ritual language
  is adopted for the co-sign/release moments inside 1a; 1b pipeline survives
  as the "under the hood" / operator view, not a doctor-facing mode.**
  Rationale: goal bar centers the doctor; the wireframe deliberately didn't
  choose, but design partners should see one opinion with the machinery one
  drawer away. → UI consolidation ticket when design partners onboard.
- **P3. Nothing removed from the demo before design partners.** The honest
  labeling (Real/Simulated in the commit, README, TODO(#n)) is the safety.

### Stop 2 — `roadmap:` (pressure test, GOAL.md, tickets #2–#12)

Disposition: **approve, no flags.** The bar table's "verified by" column is
the load-bearing piece; every row already points at a ticket or a passing
test. One recorded nit: pressure-test assertions are substring checks —
fine at this scale, replace with jq-style structural asserts if they ever
false-pass (watch for it, don't pre-engineer).

### Stop 3 — `strategy:` (investigation 0002, treatments ritual, decisions 0001/0002)

Disposition: **approve.** The operator has already engaged with all three
artifacts in-session (local-model directive, treatments directive, Liquid
staging directive) — this commit is the codification of their own calls
plus the round-1 verdict they delegated.

### Stop 4 — `staging:` (#2, real Nomad/Vault dev substrate)

Disposition: **approve with two flags** (both documented in the runbook,
neither blocking):

- **F1 (veto-able): the dev agent strips the job's `vault {}` stanza at
  submission** because dev-mode Nomad lacks workload identity; Vault is
  proven via the control plane's own transit probe instead. Cloud staging
  must keep the stanza — there's a risk the stripped path quietly becomes
  load-bearing. Mitigation queued: a pressure-test assertion that the
  *rendered* job text always contains the stanza even when submission
  strips it.
- **F2 (accepted): placement stays virtual** (`role=prod` unsatisfiable on
  one dev agent) — the test asserts registration and stop, not a running
  container. This is the honest edge of what one machine can prove; a real
  client pool is Phase 1.

### Stop 5 — `eject:` (#11, the ownership bundle)

Disposition: **approve with one flag:**

- **F3 (veto-able design call): a live app's COMPLIANCE.md re-runs preflight
  over its sandbox lineage** (synthetic view) rather than its live state,
  because a released app legitimately reads tenant data and would "fail"
  synthetic-only forever after. The alternative — freezing the attestation-
  time report verbatim instead of re-running — is arguably more honest
  evidence. Provisional: keep the re-run for the draft path, but the
  released path should embed the *frozen attestation-time report*; queued
  as a small change under #11.

### Stop 6 — `agent:` (#4, verified escalation ladder)

Disposition: **approve with one flag:**

- **F4 (known wart, ticketed thinking required): the model HTTP call is a
  blocking socket inside an async handler while holding the platform write
  lock** (bounded by 5s timeouts; inert with default config since no env
  vars = rules-only). Fine for Phase 0 single-tenant demo; unacceptable
  once a real local tier lands, because one slow model call stalls every
  request. Fix rides with #7 (Postgres state) which dissolves the
  in-memory lock, or earlier with a spawn_blocking + clone-in/merge-out
  pattern. Recorded so it cannot be forgotten: the fix must land before
  `LOCAL_MODEL_URL` is ever set in a shared environment.
  **Resolved in the #7 link** (spawn_blocking + clone-in/merge-out, the
  second of the two shapes named above): the climb runs on the blocking
  pool with no platform lock held; the apply re-acquires the lock and
  settles `concurrent-edit` if the record moved. Asserted by
  `slow_local_tier_does_not_block_a_concurrent_unrelated_request` and
  `concurrent_edit_during_climb_settles_failed_and_never_clobbers`
  (tests/ladder_contract.rs).

### Standing outcome

Six commits approved; four flags recorded (F1 pressure-test assertion, F3
frozen released-report, F4 lock-holding I/O, plus stop-2's substring-assert
watch item); three provisional product decisions (P1 dual-register gates,
P2 skin consolidation, P3 ship-as-labeled). Vetoes welcome asynchronously —
each flag names its ticket.

## #8 link — 2026-07-11, audit broker (recorded ambiently, no stop)

Disposition: built per issue #8's bar; two design calls made in the
operator's stead (rationale in decision 0004, veto-able):

- **P4: the control DB stores the Boundary-style pt/HMAC pair** for
  sensitive audit values rather than HMAC-only. Rationale: the control DB
  already holds the prompt in plaintext inside `apps.record` — it IS the
  tenant-scoped store — and the pairing is what keeps the doctor's own
  audit view plaintext across a restart. The HMAC rule governs *surfaces*
  (platform export, AUDIT_FILE archive: hash only; tenant views: words).
  Alternative (HMAC-only storage) degrades the restored tenant view to
  hashes; real at-rest envelope encryption rides #10, not this link.
- **P5: `restore` stays best-effort** while every other mutation is
  load-bearing under the broker. Rationale: sandbox-only rebuild from
  scaffold + addenda whose creation was itself durably settled; blocking it
  on a degraded sink adds doctor-visible failure without adding evidence.

Issue #8 stays open for its named remainder: object-storage archive sink,
hipaa-core runtime-event ingestion (#5), export hash-chain digests.

## #9 link — 2026-07-11, vault dynamic creds (recorded ambiently, no stop)

Disposition: built per issue #9's bar; three design calls made in the
operator's stead (veto-able, all reversible):

- **P6: staging Postgres host auth tightened to scram-sha-256** with a
  fixed, documented dev credential (`staging:staging-pg`, same spirit as
  `VAULT_TOKEN=staging-root`). Rationale: under initdb's `trust`, "issued
  creds authenticate" is vacuous (any password passes) — the #9 evidence
  must be a real password check. Idempotent tightening handles existing
  trust data dirs; CI's postgres:16 service container was already scram.
- **P7: revocation proof shape without a persisted password.** The control
  plane never persists secrets, and the pressure test's kill -9 sits
  between promote and rollback, so the platform-side revocation proof is
  login-failure as the issued user + role absent from `pg_roles` (the
  engine's revocation drops the role). The literal
  authenticate-then-revoke-then-fail with the password in hand runs in the
  pressure test via a sibling lease on the same role. Alternative (persist
  the password to keep it for rollback) rejected: it would put a live DB
  credential in the control store to strengthen a test.
- **P8: template paths aligned to the one real staging DB role.** The
  policy template and the rendered Nomad job now name
  `database/creds/tenant-app` (what exists) instead of
  `database/creds/tenant-<tenant>` (what didn't), and the job's `vault`
  stanza names the policy that is actually mounted (`tenant-<tenant>`).
  Honest labeling over aspirational paths; per-tenant DB roles are the
  Phase 1 item, stated in both artifacts.

Enforcement honesty (in the runbook, the policy file, and the commit):
dev-mode tokens are root, so the mounted per-tenant policy exists and reads
back but is not yet the enforcing credential — enforcement-by-token rides
Phase 1. Issue #9 stays open for that remainder plus per-tenant DB roles,
consul-template resolution in a scheduled container, and TTL-expiry
revocation observation (#6).

## #10 link — 2026-07-11, identity/tenancy (recorded ambiently, no stop)

Disposition: built per issue #10's bar with a Phase-0-honest shape:
config-declared principals + static bearer tokens now (labeled as the dev
credential in the file, the module doc, and the runbook), OIDC as the
designed upgrade of the token *source*, never the model. Judgment calls
made in the operator's stead (veto-able, all reversible):

- **P9: dev fallback is kept AND confessed.** With no IDENTITIES_FILE, a
  missing Authorization header resolves to dr-osei so the zero-config demo
  UI keeps working — but the first such request per boot lands an
  `auth.dev_fallback` audit event, and a *present but wrong* token is 401
  even in dev. Alternative (401 everywhere) breaks the "describe a tool,
  enjoy your day" demo loop; alternative (silent fallback) fails honest
  labeling. Staging always sets IDENTITIES_FILE → strict 401s.
- **P10: the request `tenant` field validates-equal instead of being
  removed.** Creation takes tenant from the principal; a body naming any
  other tenant is 422, matching is accepted for old clients. Removal was
  the other candidate — validate-equal was chosen because silently
  accepting-and-ignoring a field the client believes is load-bearing is a
  false-pass shape.
- **P11: the typed `cosigner` field survives only as a display-name
  check** — omit it, or match the authenticated principal's registered
  name exactly; the attestation always records the principal id + name +
  sha256(frozen report JSON) + timestamp. The blank-cosign refusal
  (pressure test) still holds: whitespace is a mismatch, not an omission.
- **P12: cross-tenant denials are audited onto the *owning tenant's* app
  stream** (actor = the denied principal, app_id = the target). The
  practice sees who knocked on their tool; the denied caller sees only the
  same 404 a nonexistent id gets.
- **P13: expired sessions reset at the 401.** With static Phase 0 tokens
  the denied request IS the logoff boundary; the next request starts a
  fresh session. Terminal expiry is only honest once credentials can
  actually expire (OIDC) — stated in the module doc rather than simulated.
- **P14: `app.iterated` keeps the `agent` actor.** The iterate route is
  tenant-guarded, so attribution is safe; threading the principal through
  the ladder adds signature churn without new enforcement. An
  "on-behalf-of principal" field on ladder events is a closeout item.
- Rollback stays role-open (staff may withdraw an app from use): safety
  beats ceremony, and the issue's role bar names promote/co-sign/export
  only.

Closeout item for the branch-convergence pass: the eval harness lives on a
sibling branch — its scenarios drive the API headerless and will ride the
dev fallback, but they need an explicit auth pass (tokens, a two-tenant
scenario, and a staff-denial expectation) when the branches merge.

Issue #10 stays open for its named remainder: OIDC token source,
NPI-verified clinician identity, operator Target/Session (decision 0005 —
design record only, absent by design until real placement exists), and a
UI login for strict instances.
