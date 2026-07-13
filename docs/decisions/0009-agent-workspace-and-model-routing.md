# Decision 0009: Gemma planning and owned output

Status: accepted for implementation

## User outcome

A clinician receives a useful application, not a chat transcript. They can
compare treatments, approve a visible source diff, run the application, export
it, and continue working without Practice Studio.

The exported application contains:

- a Svelte 5 and SvelteKit web client;
- a Rust and Axum server;
- synthetic fixtures and tests;
- editable tldraw diagrams for architecture, state, and services;
- one README that explains running, changing, testing, diagrams, Svelte MCP,
  and optional LangChain extension.

No other prose document is included.

## Service boundary

The Rust control plane owns identity, tenant scope, workspace state, source
checkpoints, diffs, validation, release gates, deployment, and audit.

The private DigitalOcean Gemma 4 agent plans two or three treatments. Its
response is untrusted data. Rust checks the response before the user can choose
a treatment. Rust then creates source from checked pack rules. Gemma cannot
write files, run commands, deploy, read production data, use GitHub, or receive
platform secrets.

Rust checks paths, size, syntax, dependencies, pack rules, tests, browser
behavior, and gate evidence before it creates a checkpoint. The user sees the
diff before acceptance.

## Model routing

Gemma 4 is the only application model. It plans treatments through a private
DigitalOcean endpoint. Deterministic pack rules create source and keep the core
workflow available when the planner is down.

## Framework comparison

We reviewed Open SWE at commit
`30832d29bcfa12c5669c374add585e8b829a8ac2` and Deep Agents as prior art. They
are comparison points, not dependencies or deployed services.

| Comparison point | Practice Studio choice | Proof |
|---|---|---|
| Repository context | Send a checkpoint digest and allowed paths, not source files | Planner request test |
| Planning state | Store the plan, the selected treatment, and agent version | Workspace persistence test |
| Tool access | Give Gemma no tools or file access | Planner request schema and DigitalOcean agent settings |
| Verification | Let Rust choose and run every check | Workspace verifier tests |
| User handoff | Export owned source, one README, and three diagrams | Export contract test |

This keeps the useful controls from those frameworks without adding a Python
worker, another state store, or another model.

## Workspace state machine

```text
described
  -> treatments_ready
  -> treatment_selected
  -> generating
  -> candidate_ready
  -> verifying
  -> review_required
  -> accepted
  -> preview_ready
  -> export_ready
```

Any generation, parsing, budget, syntax, dependency, gate, or browser failure
moves the operation to `failed` without changing the last accepted checkpoint.
Cancellation moves it to `cancelled`. A new request starts from the accepted
checkpoint, never from a failed candidate.

## Quality bar

Every candidate must:

- stay within the allowed workspace paths and byte budget;
- use Svelte 5 runes and the pack's UI contract;
- keep the server in Rust;
- compile and pass source checks;
- pass the pack fixture and browser journey;
- preserve required safety text and gate evidence;
- produce a readable diff;
- leave an immutable accepted checkpoint;
- export with the README and three editable tldraw diagrams.

The app benchmark records pass rate, repair rate, latency, Gemma use, source
size, browser outcome, and human acceptance for each treatment.
