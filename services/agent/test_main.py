import asyncio
import json
from types import SimpleNamespace

import pytest

import main as worker
from main import (
    TreatmentPlan,
    _state_file_text,
    normalize_request,
    validate_candidate_json,
    validate_treatment_json,
)


PLAN = {
    "problem": "reduce follow-up work",
    "recommended_treatment_id": "queue",
    "treatments": [
        {
            "id": "queue",
            "label": "Review queue",
            "user_outcome": "See unresolved follow-ups first.",
            "screen_changes": ["Add a review queue."],
        },
        {
            "id": "timeline",
            "label": "Patient timeline",
            "user_outcome": "Review follow-ups by patient.",
            "screen_changes": ["Add a synthetic patient timeline."],
        },
    ],
    "acceptance_checks": ["The queue uses synthetic records."],
}


def test_request_rejects_unexpected_authority():
    with pytest.raises(ValueError, match="unknown request fields"):
        normalize_request(
            {
                "schema_version": 1,
                "action": "plan",
                "task": "add a queue",
                "pack": "patient-intake",
                "api_key": "no",
            }
        )


def test_request_requires_schema_action_and_selected_treatment_for_generate():
    with pytest.raises(ValueError, match="schema_version must be 1"):
        normalize_request({"action": "plan", "task": "queue", "pack": "intake"})
    with pytest.raises(ValueError, match="action must be plan or generate"):
        normalize_request(
            {"schema_version": 1, "action": "both", "task": "queue", "pack": "intake"}
        )
    with pytest.raises(ValueError, match="selected_treatment_id is required"):
        normalize_request(
            {"schema_version": 1, "action": "generate", "task": "queue", "pack": "intake"}
        )


def test_candidate_accepts_only_workspace_paths():
    raw = json.dumps(
        {
            "summary": "Add a review queue.",
            "files": [
                {
                    "path": "web/src/routes/+page.svelte",
                    "content": "<h1>Queue</h1>",
                    "reason": "Show the queue.",
                }
            ],
            "verification_commands": ["npm run check"],
        }
    )
    assert validate_candidate_json(raw).files[0].path.endswith("+page.svelte")


def test_candidate_rejects_secret_and_deploy_paths():
    raw = json.dumps(
        {
            "summary": "Bad path.",
            "files": [
                {
                    "path": ".env",
                    "content": "TOKEN=secret",
                    "reason": "Should fail.",
                }
            ],
            "verification_commands": ["true"],
        }
    )
    with pytest.raises(ValueError, match="invalid candidate"):
        validate_candidate_json(raw)


def test_candidate_rejects_duplicate_paths():
    raw = json.dumps(
        {
            "summary": "Duplicate.",
            "files": [
                {"path": "server/src/main.rs", "content": "a", "reason": "one"},
                {"path": "server/src/main.rs", "content": "b", "reason": "two"},
            ],
            "verification_commands": ["cargo check"],
        }
    )
    with pytest.raises(ValueError, match="duplicate paths"):
        validate_candidate_json(raw)


def test_plan_rejects_unknown_recommendation_and_duplicate_ids():
    base = {
        "problem": "reduce follow-up work",
        "recommended_treatment_id": "missing",
        "treatments": [
            {"id":"one","label":"One","user_outcome":"One","screen_changes":["a"]},
            {"id":"one","label":"Two","user_outcome":"Two","screen_changes":["b"]},
        ],
        "acceptance_checks": ["works"],
    }
    with pytest.raises(ValueError):
        TreatmentPlan.model_validate(base)


def test_treatment_parser_returns_canonical_plan():
    parsed = validate_treatment_json(json.dumps(PLAN))
    assert parsed.recommended_treatment_id == "queue"
    assert [item.id for item in parsed.treatments] == ["queue", "timeline"]


def test_plan_action_returns_only_plan_and_never_builds_deep_agent(monkeypatch):
    monkeypatch.setattr(
        worker,
        "plan_treatments",
        SimpleNamespace(invoke=lambda _request: json.dumps(PLAN)),
    )
    monkeypatch.setattr(
        worker,
        "build_generation_agent",
        lambda: pytest.fail("plan must not build or invoke a Deep Agent"),
    )
    result = asyncio.run(
        worker.run_action(
            {
                "schema_version": 1,
                "action": "plan",
                "task": "add a follow-up queue",
                "pack": "patient-intake",
            }
        )
    )
    assert set(result) == {"schema_version", "treatment_plan"}
    assert result["schema_version"] == 1
    assert result["treatment_plan"]["recommended_treatment_id"] == "queue"


def test_generate_action_returns_only_candidate_after_explicit_selection(monkeypatch):
    candidate = {
        "summary": "Add a reviewed queue.",
        "files": [
            {
                "path": "web/src/routes/+page.svelte",
                "content": "<h1>Queue</h1>",
                "reason": "Show the selected treatment.",
            }
        ],
        "verification_commands": ["npm run check"],
    }

    class FakeAgent:
        async def ainvoke(self, payload, config):
            request = json.loads(payload["messages"][0].content)
            assert request["selected_treatment_id"] == "queue"
            assert config["configurable"]["thread_id"] == "thread-1"
            return {"files": {"/candidate.json": json.dumps(candidate)}}

    monkeypatch.setattr(worker, "_GENERATION_AGENT", FakeAgent())
    result = asyncio.run(
        worker.run_action(
            {
                "schema_version": 1,
                "action": "generate",
                "thread_id": "thread-1",
                "task": "add a follow-up queue",
                "pack": "patient-intake",
                "selected_treatment_id": "queue",
            }
        )
    )
    assert set(result) == {"schema_version", "candidate_patch"}
    assert result["candidate_patch"]["summary"] == "Add a reviewed queue."


def test_state_file_decoder_accepts_v2_and_rejects_binary():
    assert _state_file_text({"content":"{}","encoding":"utf-8"}, "/candidate.json") == "{}"
    with pytest.raises(ValueError, match="not a UTF-8 state file"):
        _state_file_text({"content":"e30=","encoding":"base64"}, "/candidate.json")
