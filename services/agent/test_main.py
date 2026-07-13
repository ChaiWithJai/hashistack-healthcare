import json

import pytest

from main import normalize_request, validate_candidate_json


def test_request_rejects_unexpected_authority():
    with pytest.raises(ValueError, match="unknown request fields"):
        normalize_request(
            {"task": "add a queue", "pack": "patient-intake", "api_key": "no"}
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
