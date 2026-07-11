# Investigation 0002 — Local models and ecosystem tools as footprint simplifiers

Question: can local models (Bonsai-class ternary, Liquid LFM) and ecosystem
tools (LM Studio, Unsloth) act as **major simplifiers** — minimizing our
infrastructure, compliance, and vendor footprint — while still delivering a
best-in-class experience for the job we support, at the [GOAL.md](../GOAL.md)
bar?

Related: [#4](https://github.com/ChaiWithJai/hashistack-healthcare/issues/4)
(agent driver), [#5](https://github.com/ChaiWithJai/hashistack-healthcare/issues/5)
(packs), [#12](https://github.com/ChaiWithJai/hashistack-healthcare/issues/12)
(use-case enablement) · Surveyed July 2026.

## Answer in one paragraph

Yes — but as **routing and packaging decisions, not a wholesale swap**. Local
models can't replace the frontier model where the product wins or loses trust
(first scaffold generation, hard iterations, reviewer notes), and the vendors
say so themselves: Liquid explicitly warns against using LFM2.5-class models
for code generation. But three layers of our stack get dramatically simpler:
(1) the **iterate loop** can route its constrained, scaffold-shaped edits to a
mid-size open-weight coder (Qwen3-Coder-30B-class, ~77–80% SWE-bench — within
a few points of frontier) served inside our own VPC, shrinking per-token cost
and the third-party AI BAA surface toward zero; (2) the **local runtime
profile** (Wave 4) is suddenly *early*, not late — LFM2.5 and ternary
Bonsai-class 8B models run extraction/de-id/classification on a clinician's
own hardware, which is both the smallest possible footprint and the strongest
possible compliance story (PHI never leaves the device); and (3) the **pack**
becomes the unit that carries its own small tuned model (Unsloth QLoRA on
synthetic data → GGUF), which extends "no hostage code" to "no hostage
model." The deepest simplifier isn't any model — it's that **the pack system
shrinks the job the model must do**: the more the scaffold constrains, the
smaller the model that clears the bar. Investment in #5 *is* the
model-shrinking strategy.

## What we evaluated (state as of July 2026)

| Thing | What it is now | Fits where |
|---|---|---|
| **Bonsai-class ternary** ([deepgrove Bonsai 0.5B](https://github.com/deepgrove-ai/Bonsai); [PrismML Ternary Bonsai 8B/4B/1.7B](https://prismml.com/news/ternary-bonsai), 1.58-bit, ~9× memory cut, GGUF + MLX 2-bit builds) | Ultra-efficient weights; 8B ternary ≈ phone/laptop class | Local profile packs: de-id (17), note extraction (18), airgapped (19) |
| **Liquid LFM2.5** ([230M runs ~213 tok/s on phone CPU](https://www.liquid.ai/blog/lfm2-5-230m); [8B-A1B on-device MoE](https://www.liquid.ai/blog/lfm2-5-8b-a1b); llama.cpp/MLX/vLLM/ONNX; LEAP deployment; open-weight) | Instruction-following/agentic on-device family; **vendor states: not for code generation**; 8B-A1B "falls short on heavy codegen" | Local profile + on-device pack UX (form logic, extraction, classification) — never the builder |
| **Qwen3-Coder-30B-class open-weight coders** ([30B MoE, 3.3B active, 256K ctx](https://www.kdnuggets.com/top-7-coding-models-you-can-run-locally-in-2026); Devstral 24B for agentic loops; best open ~[80% SWE-bench vs Claude Opus 4.7 at 87.6%](https://dev.to/danishashko/the-best-llms-for-agentic-coding-in-2026-real-world-not-just-benchmarks-96n)) | The routing tier that matters for us: industry pattern is 60–80% of agent traffic local, hard 20% escalated to frontier | In-VPC `LocalDriver` for scaffold-constrained iterate edits (#4) |
| **LM Studio** ([llmster daemon = headless server](https://lmstudio.ai/docs/developer), py/ts SDKs, OpenAI **and Anthropic-compatible** endpoints, MCP client, [enterprise tier with centralized model/plugin controls](https://lmstudio.ai/enterprise)) | The local runtime we don't have to build — and the tool clinician-builders already have | Supported runtime target for ejected local packs + clinician devices; **not** our pool serving layer |
| **Unsloth** ([QLoRA 8B on a 12GB consumer GPU; built-in GGUF export](https://www.sitepoint.com/fine-tune-local-llms-2026/) straight into llama.cpp/Ollama/LM Studio) | The domain-tuning toolchain | Per-pack tuned models, trained **only on synthetic data** (Synthea) — compliance-clean by construction |

## Where the footprint actually shrinks

1. **Compliance surface (the big one).** Today's architecture needs BAAs with
   DO + Anthropic (+ a voice vendor in Wave 3). Routing iterate-loop traffic
   to an in-VPC model removes third-party AI exposure for the bulk of tokens;
   the local profile removes *us* from the data path entirely. The
   `ai-allowlist` gate stops being a control we enforce and becomes a
   property of the topology. For hospital sales, "PHI never leaves your
   building" beats any BAA stack we could assemble.
2. **Infra.** The local profile needs **zero allocations** — the platform's
   deliverable shrinks to signing, gates, docs, and an optional non-PHI
   sidecar. The in-VPC routing tier is one modest GPU node (or CPU-tolerable
   MoE — 3.3B active params) as a Nomad job on the prod pool: versioned,
   Packer-imaged, immutable, exactly like everything else. No GPU fleet.
3. **Cost.** Per-token API spend for the chatty middle of the workflow (the
   iterate loop is most of the tokens) drops to electricity. Frontier spend
   concentrates where it buys trust.
4. **Vendor.** `AgentDriver` + an OpenAI-compatible client covers vLLM,
   llama.cpp, and LM Studio with one driver. GGUF is the portability floor.
   No model lock-in, mirroring "no hostage code."

## Where we must NOT simplify (the bar guards)

- **First generation is the product's first impression.** A doctor describes
  their tool and watches it appear. A 30B local model that fumbles that
  moment kills trust permanently. Scaffold authorship, ambiguous iterations,
  and the platform reviewer's release note stay on the frontier model
  (Claude), under BAA. Route by task, never by ideology.
- **Escalation must be automatic and invisible.** The doctor never picks a
  model (Tao 1: workflows, not technologies). `LocalDriver` failures — gate
  regressions, compile failures, low-confidence edits — escalate silently to
  `ClaudeDriver` and both paths land identically in the audit stream.
- **Small on-device models are not builders.** LFM/Bonsai-class models do
  extraction, classification, form logic, de-id — the *runtime brains inside
  shipped packs* — not code generation. The vendors say so; believe them.

## The compounding insight

The RFC's pack system and the local-model trend multiply each other:

> **Every constraint a pack adds is model capability we don't have to buy.**

A blank-canvas builder needs 87% SWE-bench. A pack that pre-wires hipaa-core,
fixes the data model, and constrains edits to well-shaped slots needs far
less — and a pack that ships its own Unsloth-tuned 4B for its runtime job
needs no cloud at all. The wave plan's economics improve with every pack we
harden, which is the opposite of the usual "capability tax" curve.

## Decisions (proposed, with the 0001 rubric applied)

| # | Decision | Call |
|---|---|---|
| D1 | Agent driver architecture (#4) | Ship `AgentDriver` as a **router**: `ClaudeDriver` (scaffold authorship, hard edits, review) + `LocalDriver` (OpenAI-compatible endpoint → vLLM/llama.cpp/LM Studio interchangeable) with automatic escalation. Rule-based driver stays for CI. |
| D2 | Pool model serving | vLLM or llama.cpp **as a Nomad job** on our pools (codifiable, immutable, auditable). LM Studio stays an *edge/eject target*, not pool infrastructure — its enterprise tier is a clinic-side convenience, not our substrate. |
| D3 | Pack spec (#5) grows an optional `model {}` block | Source (HF ref), quantization, runtime targets (llama.cpp / LM Studio / LEAP), tuning provenance (Unsloth config + **synthetic** dataset hash), all covered by the pack signature. A model is pack content, like prompts. |
| D4 | Wave plan (#12) | Pull deid-local (17) and note-extraction-local (18) **forward** — the model substrate matured faster than the RFC assumed, and they're the strongest compliance story we can demo a hospital. |
| D5 | Eject (#11) | Ejected local packs must run without us: GGUF + llama.cpp instructions as the floor, LM Studio as the friendly path. "No hostage model." |

Rubric check: D1–D5 leave the doctor's workflow untouched (✓ Tao), every
routing decision lands in the audit stream (✓ evaluable), the sandbox
boundary is unaffected (✓), margin moves in-house where rented tokens were
the cost (✓), and eject gets cleaner, not dirtier (✓).

## Cheapest proofs (in priority order)

1. **Routing spike** (folds into #4): pack-constrained iterate tasks from the
   Wave 1 packs run against Qwen3-Coder-30B-class via an OpenAI-compatible
   endpoint; measure gate-pass rate and escalation rate vs Claude-only.
   Kill threshold: <70% local gate-pass on constrained edits.
2. **De-id pack spike** (pulls Wave 4 forward): LFM2.5-1.2B or Ternary-Bonsai
   quantized, running a de-id pass over Synthea notes on a laptop via
   LM Studio and via llama.cpp; ship as the first `model {}`-bearing pack.
3. **Unsloth loop**: tune a 4B on synthetic intake notes → GGUF → the same
   pack, measuring the tuned-vs-base delta to know if per-pack tuning earns
   its complexity.
