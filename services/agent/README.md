# Practice Studio builder worker

This service runs the source-generation worker on the DigitalOcean Agent
Development Kit.

It uses:

- Qwen3 Coder Flash for the tool-calling supervisor;
- Gemma 4 for treatment planning;
- GPT-5.6 Sol for bounded Svelte and Rust source generation.

The worker has no deployment, GitHub, production, or patient-data tools. It
returns a candidate workspace. The Rust control plane validates and checkpoints
that candidate before the user can accept it.

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
