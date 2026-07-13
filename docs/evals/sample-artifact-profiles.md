# Review of 12 exported apps

This review samples 12 of the 17 exported apps. The harness built each app,
started it, and completed its browser journey on 2026-07-13. The apps use
synthetic data. This review does not claim that they are ready for patient data.

## Result

All 12 apps completed the tested user job. Each export contained Rust source,
Svelte 5 source, a lockfile, one README, and three editable tldraw diagrams.
Each export was between 124 KiB and 196 KiB.

The old review gave every export 5 out of 5. That score was too generous. The
deployed app runs the Rust interface. It does not build or serve the included
Svelte interface. The README also tells the owner to run `cd app`, but no such
directory exists. It points to `server/assets/clinician.css`, while the file is
at `web/src/clinician.css`.

| App | Tested user outcome | Job | Continue | Export |
|---|---|---:|---:|---:|
| Post operative monitor | A high pain check in creates a practice flag | 5 | 3 | 3 |
| Hypertension tracker | An urgent reading enters the clinician alert path | 5 | 3 | 3 |
| Patient intake | A submitted intake becomes a chart summary | 5 | 3 | 3 |
| Insurance verification | An eligibility result enters a review queue | 4 | 3 | 3 |
| Patient portal | A patient sees only their record and queues a message | 4 | 3 | 3 |
| Inbound scheduling | Urgent language is held for staff review | 4 | 3 | 3 |
| Outbound follow up | Consent and concerning replies change the workflow | 4 | 3 | 3 |
| RPM wearables | A device observation enters a human review queue | 4 | 3 | 3 |
| Visit notes | Transcript segments create an unsigned draft | 4 | 3 | 3 |
| Local deidentification | Local rules create a reviewable redaction draft | 4 | 3 | 3 |
| Air gapped support | Bundled reference search works without a network | 4 | 3 | 3 |
| Hybrid pipeline | A disclosure preview shows that nothing was sent | 4 | 3 | 3 |

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
| Verification | 4 out of 5 | Rust and browser checks pass, but they do not prove the included Svelte app |
| User handoff | 3 out of 5 | The export is small and owned, but its two source trees are not one runnable product |

Open SWE uses a task sandbox, curated tools, repository context, and review
feedback. Deep Agents packages planning, file access, context management, and
subagents. Our product does not need those runtimes for treatment planning. We
do need the same clear link between context, verification, and the final
handoff.

## Next fixes

1. Build the Svelte app and serve it through the Rust application and Docker
   image.
2. Make the browser journey test that same Svelte and Rust product.
3. Fix the two README paths and add a check that every command and path in the
   README exists.
4. Test one first customization, export it again, and import it as a new pack.

## Proof

The full run passed 458 of 458 platform checks and 432 of 432 artifact checks
across 78 scenarios. The machine results are in
[scorecard.json](scorecard.json). Screenshots are in the
[screenshots](screenshots/) directory. The structured sample is in
[sample-artifact-profiles.json](sample-artifact-profiles.json).
