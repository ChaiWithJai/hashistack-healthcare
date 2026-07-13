# Practice Studio builder worker

This service runs the source-generation worker on the DigitalOcean Agent
Development Kit.

It uses:

- Qwen3 Coder Flash for the tool-calling supervisor;
- Gemma 4 for treatment planning;
- GPT-5.6 Sol for bounded Svelte and Rust source generation.

The worker has no deployment, GitHub, production, or patient-data tools. Its
version-1 protocol has two separate actions. They cannot be combined:

- `plan` calls Gemma 4 directly and returns only `schema_version` and
  `treatment_plan`. It never constructs or invokes a Deep Agent and cannot call
  the source generator.
- `generate` requires the treatment ID that the user selected. It uses the
  bounded Open SWE-style generation harness and returns only `schema_version`
  and `candidate_patch`.

The Rust control plane stores the plan, records the explicit selection,
validates the untrusted candidate, computes its diff, and checkpoints it only
after the user accepts it.

## Protocol

Plan request:

```json
{"schema_version":1,"action":"plan","thread_id":"app-123","task":"add a follow-up queue","pack":"patient-intake","workspace_summary":"accepted checkpoint v1"}
```

Plan response:

```json
{"schema_version":1,"treatment_plan":{"problem":"...","recommended_treatment_id":"queue","treatments":["2 or 3 typed treatments"],"acceptance_checks":["..."]}}
```

Generation request:

```json
{"schema_version":1,"action":"generate","thread_id":"app-123","task":"add a follow-up queue","pack":"patient-intake","workspace_summary":"accepted checkpoint v1","selected_treatment_id":"queue"}
```

Generation response:

```json
{"schema_version":1,"candidate_patch":{"summary":"...","files":["bounded typed file changes"],"verification_commands":["advisory commands"]}}
```

Unknown fields, unsupported schema versions or actions, and a generation
request without `selected_treatment_id` are rejected. Model-proposed
verification commands remain advisory; the Rust control plane chooses and runs
the real checks.

## Local check

```bash
python3 -m venv .venv
. .venv/bin/activate
pip install -r requirements.txt pytest
pytest -q
```

Live model calls require DigitalOcean inference configuration and an OpenAI
provider key for GPT-5.6 Sol.

## DigitalOcean ADK

```bash
gradient agent run
gradient agent deploy
```

The deployment must set:

- `PRACTICE_STUDIO_PLANNER_MODEL=gemma-4-31B-it`
- `PRACTICE_STUDIO_GENERATOR_MODEL=openai-gpt-5.6-sol`
- `PRACTICE_STUDIO_ORCHESTRATOR_MODEL=qwen3-coder-flash`

The model and framework choices are recorded in decision 0009.
