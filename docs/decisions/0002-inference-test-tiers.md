# Decision 0002: Retired inference test tiers

Status: superseded by [decision 0009](0009-agent-workspace-and-model-routing.md).

This record preserves an experiment that is no longer part of the product.
The experiment compared small local models with fixed test responses. It also
tested whether a separate model server would help staging.

Practice Studio does not deploy Liquid, Hermes, llama.cpp, a frontier model,
or a second model worker. Gemma 4 is the only application model. Hosted staging
uses the private DigitalOcean Gemma agent. Local checks use deterministic Rust
behavior or a bounded Gemma response fixture.

The old model results remain in `docs/investigations` as historical evidence.
They do not define the current architecture or a future deployment plan.

## Current test levels

| Test level | Model behavior | Purpose |
|---|---|---|
| Rust tests | Fixed local Gemma response fixtures | Check parsing, rejection, fallback, and provenance without a network call |
| Local product flow | Deterministic Rust planner | Keep the full build and export flow available without a hosted service |
| DigitalOcean staging | Private Gemma 4 agent | Measure the same bounded planning call used by the hosted product |

Tests must not select a second model through an environment variable. The Rust
service accepts one hosted provider and checks that it reports the exact Gemma
model name. A planner failure uses the deterministic Rust path and records the
reason.

Related records are [decision 0009](0009-agent-workspace-and-model-routing.md)
and [the retired local model investigation](../investigations/0006-liquid-hermes-coding-floor.md).
