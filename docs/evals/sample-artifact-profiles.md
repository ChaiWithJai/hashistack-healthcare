# Review of 12 exported apps

This review samples 12 of the 17 exported apps. The harness built each app,
started it, and completed its browser journey on 2026-07-13. The apps use
synthetic data. This review does not claim that they are ready for patient data.

## Result

All 12 apps completed the tested user job. Each export contained Rust source,
Svelte 5 source, locked dependencies, one README, and three tldraw diagrams.
Each export was between 124 KiB and 196 KiB.

PR 52 fixed the split product. The exported Docker image now builds both
source trees, serves the clinical workflow and customization workspace from
one origin, and uses paths that exist. CI drives the clinical workflow and
the connected Svelte workspace in that image.

The remaining limit is deeper. The Svelte workspace checks Rust health, but
its editable checklist does not perform the pack's clinical job through Rust.
The export is runnable, while the first meaningful Svelte and Rust
customization still needs end to end proof.

| App | Tested user outcome | Job | Continue | Export |
|---|---|---:|---:|---:|
| Post operative monitor | A high pain check in creates a practice flag | 5 | 3 | 4 |
| Hypertension tracker | An urgent reading enters the clinician alert path | 5 | 3 | 4 |
| Patient intake | A submitted intake becomes a chart summary | 5 | 3 | 4 |
| Insurance verification | An eligibility result enters a review queue | 4 | 3 | 4 |
| Patient portal | A patient sees only their record and queues a message | 4 | 3 | 4 |
| Inbound scheduling | Urgent language is held for staff review | 4 | 3 | 4 |
| Outbound follow up | Consent and concerning replies change the workflow | 4 | 3 | 4 |
| RPM wearables | A device observation enters a human review queue | 4 | 3 | 4 |
| Visit notes | Transcript segments create an unsigned draft | 4 | 3 | 4 |
| Local deidentification | Local rules create a reviewable redaction draft | 4 | 3 | 4 |
| Air gapped support | Bundled reference search works without a network | 4 | 3 | 4 |
| Hybrid pipeline | A disclosure preview shows that nothing was sent | 4 | 3 | 4 |

Job means that the browser completed the stated synthetic workflow. Continue
means that a new owner can find and make the next safe change. Export means that
the repository runs as one product and can be deployed or imported elsewhere.

## What the framework review changed

We compared our system with Open SWE and Deep Agents. We did not install or
deploy either framework.

| Comparison point | Result | Evidence |
|---|---:|---|
| Repository context | 3 out of 5 | Gemma receives a small checkpoint summary, but it cannot inspect the owned source |
| Planning state | 4 out of 5 | Rust stores the plan, the selected treatment, and the Gemma version |
| Tool isolation | 5 out of 5 | Gemma has no shell, file, GitHub, or deployment tools |
| Verification | 5 out of 5 | CI builds one Docker image and drives the Rust workflow and connected Svelte workspace |
| User handoff | 4 out of 5 | The export runs as one product, but a new owner has not completed the full customize, rebuild, reimport, and deploy exercise |

Open SWE uses a task sandbox, curated tools, repository context, and review
feedback. Deep Agents packages planning, file access, context management, and
subagents. Our product does not need those runtimes for treatment planning. We
do need the same clear link between context, verification, and the final
handoff.

## Next fixes

1. Make one post operative treatment change the Svelte screen, Rust behavior,
   synthetic fixture, and browser contract together.
2. Run that exact changed workflow in the studio preview and exported image.
3. Ask a new owner to customize, rebuild, reimport, and deploy the export by
   following only its README.

## Proof

The full run passed 458 of 458 platform checks and 432 of 432 artifact checks
across 78 scenarios. The machine results are in
[scorecard.json](scorecard.json). Screenshots are in the
[screenshots](screenshots/) directory. The structured sample is in
[sample-artifact-profiles.json](sample-artifact-profiles.json).
