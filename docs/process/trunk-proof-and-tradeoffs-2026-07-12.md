# Local trunk proof and open tradeoffs

## Decision

The integrated local tree is a strong learning product and artifact generator.
It is not ready to handle patient data in production.

Doctors can describe a small tool, change it, see why release is blocked, fix
the controls, review the result, publish a synthetic demo, and export source
that they can build and change. The local proof covers 17 app packs and 78
user scenarios.

The open work is concentrated in production identity, complete deployment
recovery, durable infrastructure tests, and proof that a new user can change
and share an exported app without help.

## What passed on this machine

The command below passed on July 12, 2026:

```sh
ALLOW_LARGE_PR=1 MERGE_BASE=HEAD ./scripts/merge-gate.sh --full
```

The results were:

| Check | Result |
|---|---:|
| Rust platform tests | 90 passed |
| Standalone pack tests | 49 passed |
| Local staging pressure checks | 89 passed |
| User workflow checks | 458 of 458 passed |
| Built artifact checks | 432 of 432 passed |
| App packs | 17 |
| Evaluation scenarios | 78 |

The first staging run did not configure Nomad, Vault, or Postgres. A second run
used all three services and passed 130 of 130 pressure checks. It proved job
submission, allocation execution, HTTP health traffic, dynamic database
credentials, key rotation, durable restart, and rollback.

## Repairs made during the trunk review

The operation ladder now writes a RUNNING operation to the control database
before any model or rules driver starts. If the database refuses that write,
the platform stops the operation.

An audit failure now restores an app only when the app still matches the exact
state installed by that request. This prevents a slow audit failure from
erasing a newer doctor action.

If Vault issues a database lease and Nomad then refuses the deployment, the
platform revokes the new lease. If revocation also fails, the response names
both failures.

The repository now ignores target directories created by standalone pack
builds. The merge gate now examines 233 source and evidence files instead of
17,281 generated build files.

## Product value that is ready to show

The current demo supports the main learning loop:

1. A doctor chooses one of 17 practical app types.
2. The doctor describes the change in ordinary language.
3. The platform creates a synthetic sandbox.
4. The gate report shows passed controls, failed controls, and labeled stubs.
5. The doctor can fix supported controls and request review.
6. The doctor can publish a synthetic demo without granting access to patient data.
7. The doctor can export a Rust app with source, tests, a runbook, compliance evidence, a Nomad job, and `docs/CUSTOMIZE.md`.

The strongest audience artifacts are the app profile report, the scorecard,
the five step journey, and the screenshots of all 17 running exports.

## Tradeoffs that remain open

| Area | Current choice | Benefit | Missing proof or cost | Decision needed |
|---|---|---|---|---|
| App generation | Continuous integration uses a deterministic rules driver. | Tests are repeatable and do not send clinical text to a model. | The test suite does not measure model quality or model failure recovery. | Choose supported local and hosted model tiers, then add quality and privacy tests for each tier. |
| Release safety | Labeled stubs can publish only to a synthetic demo pool. | People can show unfinished learning apps without patient access. | The user interface must keep the synthetic limit clear at every entry point. | Keep this exception only if user sessions confirm that doctors understand it. |
| Identity | Local bearer tokens and a small identity registry support the demo. | Local setup is simple and tenant tests are repeatable. | Tokens are reusable. There is no production login, strong revocation, or operator session system. | Select an identity provider and add short lived workload and user credentials. |
| Attestation | The record binds the clinician identity to a hash of the frozen gate report. | Anyone can detect a changed report. | A hash is not a signature. It does not prove who signed the report outside this database. | Add a signing service or hardware backed key and define verification and key rotation. |
| Deploy recovery | Promotion revokes a Vault lease when Nomad submission fails. | One known credential leak path is closed. | Rollback can stop a job and then fail to revoke its lease. The current API cannot describe this partial result without risking a false state. | Add an explicit recovering state and a retryable compensation operation. |
| Workload identity | Staging can mount a tenant Vault policy and issue dynamic database credentials. | The pressure path can prove credential issue and revocation. | The development root token remains the enforcing credential. | Use Nomad workload identity with a separate Vault role for each tenant or app. |
| Durable state | Postgres stores apps, operation rows, transitions, and audit events. | The forced kill and restart test restored the live app and its release record. | Some nonstage writes still accept degraded database durability. | Decide which actions require database acknowledgement and fail every required action closed. |
| Audit | The broker requires a durable sink for important actions and protects sensitive values with HMAC. | A sink failure can block release and the platform can export a safe audit view. | Object storage retention, runtime event collection, archive integrity, and independent verification are incomplete. | Define retention and tamper evidence, then test restore from the archive. |
| Pack ownership | Every export includes runnable Rust source and a quality contract. | A doctor can leave the managed service with working code. | Most packs do not yet have the full prompt, policy, gate, citation, and signature folder layout. | Set one required pack format and migrate all 17 packs. |
| Customization | Every export includes `docs/CUSTOMIZE.md` and a derived pack that the platform can parse again. | The bundle explains where to change the app and how to rerun tests. | No new user has completed a clean room change and shared or imported the result. | Run five observed handoff sessions and record time, failures, and help required. |
| Runtime profiles | Web, stream, and local app examples exist. | The catalog can show several clinic settings. | Stream apps do not yet prove queue drain or voice ingestion. Local apps do not yet prove offline model bundles or zero network use. | Write separate release bars and architecture decisions for each profile. |
| Merge shape | The integrated tree proves the full story in one place. | Cross feature failures are visible before the work is split. | The tree has 233 changed files and about 30,700 changed lines. No reviewer can safely approve it as one pull request. | Split the tree into platform contracts, packs, evaluations, and clinician experience pull requests. |

## Recommended next proof

Run the Nomad, Vault, and Postgres staging path on the same commit and save the
machine readable output. Then run a clean room session in which a doctor
exports one app, changes one clinical workflow, reruns its contract, and
imports or shares the changed pack. These two proofs cover the largest gap
between the current demo and the stated goal.

## Evidence

* `docs/evals/local-infrastructure-proof-2026-07-12.md` contains the Nomad, Vault, and Postgres proof.
* `docs/evals/scorecard.md` contains all scenario results.
* `docs/evals/sample-artifact-profiles.md` compares the sampled exports.
* `docs/evals/journey/journey.md` shows the doctor flow.
* `docs/process/merge-readiness-2026-07-12.md` records the pull request review.
* `docs/hashicorp-steering.md` records the HashiCorp patterns used for the design.
