# Gemma signed recipe profile

We tested ten packs against DigitalOcean staging on 2026-07-13. Every request
used synthetic data.

## Result

The old contract asked Gemma to write three complete treatments. Seven of ten
requests succeeded. Two responses failed the schema check, and one request
timed out. The median request took 36 seconds.

The new contract gives Gemma three recipes from the signed pack and asks for
one recipe ID. All ten requests used DigitalOcean Gemma. Every response used
the exact model name and prompt version. No request used the fallback. The
median request took 3.306 seconds.

| Measure | Before | After |
|---|---:|---:|
| DigitalOcean Gemma responses | 7 of 10 | 10 of 10 |
| Fallbacks | 3 | 0 |
| Responses inside the signed recipe contract | not applicable | 10 of 10 |
| Median request time | 36 seconds | 3.306 seconds |
| Slowest request | 45 seconds | 3.873 seconds |

The two packs that had schema failures now pass. The pack that timed out also
passes. Rust rebuilds every full treatment from the signed pack, so the model
cannot promise a field or workflow that the exporter does not implement.

The export contains one Svelte component with a guided worklist, an event
timeline, and a focused task view. The selected recipe changes which structure
the clinician sees. The workspace verifier rejects an unknown recipe or a
missing materializer.

## Architecture comparison

Open SWE and Deep Agents are reference designs only. We did not install or
deploy either framework.

| Comparison point | Practice Studio choice |
|---|---|
| Context | Gemma receives the task and bounded pack context. It does not receive source files. |
| State | Rust stores the plan, selection, version, checkpoint, and review state. |
| Tools | Gemma has no file, shell, GitHub, deployment, or data tools. |
| Verification | Rust chooses the checks and rejects work that does not match a signed recipe. |
| Handoff | The clinician exports owned Rust and Svelte source with the selected recipe. |

This is a smaller system than either reference framework. It fits the product
because Gemma makes one bounded choice and Rust owns every action with side
effects.

## Reproduce

Run the checked script against local development or staging.

```bash
scripts/profile-gemma-planner.sh http://127.0.0.1:3000
REQUIRE_GEMMA=1 scripts/profile-gemma-planner.sh https://138-197-27-225.sslip.io
```

The script starts a separate anonymous synthetic workspace for every pack. It
records provider, model, prompt version, fallback, elapsed time, recipe IDs,
and the selected recipe as JSON lines.

The machine results are in
[gemma-signed-recipe-profile-2026-07-13.json](gemma-signed-recipe-profile-2026-07-13.json).

## Limit

The first run did not save its exact task text. Both runs used the same ten
packs and the same task intent. The time comparison is therefore directional,
not a controlled benchmark. The checked script now fixes the task text for
future runs.

This change proves bounded planning and visible export materialization. It does
not yet prove that a clinician can run the selected treatment inside the studio
before export. That remains the next product gap.
