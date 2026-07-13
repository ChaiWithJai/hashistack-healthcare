# Decision 0009: Open SWE worker, bounded models, and owned output

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

A Python worker runs on the DigitalOcean Agent Development Kit. It follows the
Open SWE composed-harness pattern on Deep Agents and LangGraph. Each request
uses thread-local state and a curated tool set. The worker can plan treatments
and propose a bounded source patch. It cannot deploy, read production data, use
GitHub, read host files, or receive platform secrets.

The worker result is untrusted. Rust checks paths, size, syntax, dependencies,
pack rules, tests, browser behavior, and gate evidence before it creates a
checkpoint. The user sees the diff before acceptance.

Open SWE upstream is pinned for architecture review at commit
`30832d29bcfa12c5669c374add585e8b829a8ac2`. The DigitalOcean ADK template
reference is pinned at `825656a3f7725ac01061f5777cbc33d767c24a41`.
Practice Studio composes on their public interfaces and patterns. It does not
vendor either repository.

## Model routing

| Task | Model | Reason |
|---|---|---|
| Treatment planning and review | `gemma-4-31B-it` | Low cost and good structured reasoning without filesystem authority |
| Source boilerplate | `openai-gpt-5.6-sol` | High-quality bounded generation through a DigitalOcean provider key |
| Tool-calling supervision | `qwen3-coder-flash` | DigitalOcean lists coding and tool-calling support |
| Vision extraction | `LFM2.5-VL-1.6B` | Liquid recommends it for vision tasks |
| Audio input and response | `LFM2.5-Audio-1.5B` | Liquid recommends it for speech and audio |
| Offline fallback | deterministic pack rules | The core workflow must still work without a model |

Gemma 4 is not the filesystem supervisor because the live DigitalOcean catalog
does not list tool calling for that model. GPT-5.6 Sol is not allowed to write
files directly. It returns a typed candidate patch.

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

The 15 to 22 app benchmark records pass rate, repair rate, latency, model use,
cost, source size, browser outcome, and human acceptance for each treatment.
