"""DigitalOcean ADK worker for bounded Practice Studio source generation.

The worker follows Open SWE's composed harness pattern without giving a model
host filesystem, GitHub, deployment, or production credentials. It returns a
candidate workspace. The Rust control plane remains the authority that
validates, checkpoints, and exposes a diff to the user.
"""

from __future__ import annotations

import json
import os
from typing import Any

from deepagents import create_deep_agent
from deepagents.backends import StateBackend
from gradient_adk import entrypoint
from langchain_core.messages import HumanMessage
from langchain_core.tools import tool
from langchain_gradient import ChatGradient
from pydantic import BaseModel, Field, ValidationError


PLANNER_MODEL = os.getenv("PRACTICE_STUDIO_PLANNER_MODEL", "gemma-4-31B-it")
GENERATOR_MODEL = os.getenv(
    "PRACTICE_STUDIO_GENERATOR_MODEL", "openai-gpt-5.6-sol"
)
ORCHESTRATOR_MODEL = os.getenv(
    "PRACTICE_STUDIO_ORCHESTRATOR_MODEL", "qwen3-coder-flash"
)
MAX_SOURCE_FILES = int(os.getenv("PRACTICE_STUDIO_MAX_SOURCE_FILES", "64"))
MAX_SOURCE_BYTES = int(os.getenv("PRACTICE_STUDIO_MAX_SOURCE_BYTES", "524288"))


class Treatment(BaseModel):
    id: str = Field(pattern=r"^[a-z0-9-]{1,48}$")
    label: str = Field(min_length=1, max_length=80)
    user_outcome: str = Field(min_length=1, max_length=240)
    screen_changes: list[str] = Field(min_length=1, max_length=8)
    data_changes: list[str] = Field(default_factory=list, max_length=8)
    safety_notes: list[str] = Field(default_factory=list, max_length=8)


class TreatmentPlan(BaseModel):
    problem: str = Field(min_length=1, max_length=400)
    recommended_treatment_id: str
    treatments: list[Treatment] = Field(min_length=2, max_length=3)
    acceptance_checks: list[str] = Field(min_length=1, max_length=12)


class FileChange(BaseModel):
    path: str = Field(pattern=r"^(web|server|tests|synthetic)/[A-Za-z0-9_./+-]+$")
    content: str
    reason: str = Field(min_length=1, max_length=200)


class CandidatePatch(BaseModel):
    summary: str = Field(min_length=1, max_length=300)
    files: list[FileChange] = Field(min_length=1)
    verification_commands: list[str] = Field(min_length=1, max_length=12)

    def enforce_budget(self) -> "CandidatePatch":
        if len(self.files) > MAX_SOURCE_FILES:
            raise ValueError(f"candidate exceeds {MAX_SOURCE_FILES} files")
        total = sum(len(item.content.encode("utf-8")) for item in self.files)
        if total > MAX_SOURCE_BYTES:
            raise ValueError(f"candidate exceeds {MAX_SOURCE_BYTES} bytes")
        if len({item.path for item in self.files}) != len(self.files):
            raise ValueError("candidate contains duplicate paths")
        return self


def _json(value: BaseModel) -> str:
    return value.model_dump_json(indent=2)


def _model(name: str) -> ChatGradient:
    return ChatGradient(model=name, temperature=0)


@tool
def plan_treatments(request: str) -> str:
    """Create two or three distinct screen-level treatments for one user task."""

    planner = _model(PLANNER_MODEL).with_structured_output(TreatmentPlan)
    result = planner.invoke(
        [
            (
                "system",
                "You design calm clinical software. Return two or three concrete "
                "treatments. Each treatment must name what changes on screen, what "
                "data changes, and what safety boundary remains visible. Prefer "
                "ordinary language and synthetic data.",
            ),
            ("user", request),
        ]
    )
    return _json(TreatmentPlan.model_validate(result))


@tool
def generate_workspace_patch(request: str) -> str:
    """Generate a bounded Svelte 5 frontend and Rust server source patch."""

    generator = _model(GENERATOR_MODEL).with_structured_output(CandidatePatch)
    result = generator.invoke(
        [
            (
                "system",
                "Generate a candidate patch for an owned clinical tool. The web "
                "client uses Svelte 5 runes and SvelteKit. The server uses Rust and "
                "Axum. Use convention over configuration. Keep all examples "
                "synthetic. Do not create deployment credentials, prose documents, "
                "or package lockfiles. Return complete file contents only under "
                "web/, server/, tests/, or synthetic/.",
            ),
            ("user", request),
        ]
    )
    candidate = CandidatePatch.model_validate(result).enforce_budget()
    return _json(candidate)


SYSTEM_PROMPT = """You are the Practice Studio source-workspace supervisor.

Follow the Open SWE composed-harness pattern:
1. Write a short todo list.
2. Call plan_treatments exactly once.
3. Select the recommended treatment unless the request names another treatment.
4. Call generate_workspace_patch exactly once with the selected treatment,
   current workspace summary, pack constraints, and acceptance checks.
5. Save the treatment plan to /treatment.json and the candidate patch to
   /candidate.json using the thread-local StateBackend.
6. Return a short JSON object with treatment_file, candidate_file, and summary.

Never claim that code compiled or passed a browser check. The Rust control
plane performs those checks after this worker returns. Never request secrets,
patient data, GitHub access, shell access, or deployment access."""


_AGENT = None


def build_agent():
    return create_deep_agent(
        model=_model(ORCHESTRATOR_MODEL),
        system_prompt=SYSTEM_PROMPT,
        tools=[plan_treatments, generate_workspace_patch],
        backend=StateBackend,
    )


def normalize_request(data: dict[str, Any]) -> dict[str, Any]:
    allowed = {
        "thread_id",
        "task",
        "pack",
        "workspace_summary",
        "selected_treatment_id",
    }
    unknown = sorted(set(data) - allowed)
    if unknown:
        raise ValueError(f"unknown request fields: {', '.join(unknown)}")
    task = str(data.get("task", "")).strip()
    pack = str(data.get("pack", "")).strip()
    if not task or not pack:
        raise ValueError("task and pack are required")
    return {
        "thread_id": str(data.get("thread_id", "")).strip() or "ephemeral",
        "task": task,
        "pack": pack,
        "workspace_summary": str(data.get("workspace_summary", "")).strip(),
        "selected_treatment_id": str(
            data.get("selected_treatment_id", "")
        ).strip(),
    }


@entrypoint
async def main(data: dict[str, Any], context: dict[str, Any]):
    """Run one bounded generation turn in the DigitalOcean ADK."""

    del context
    request = normalize_request(data)
    prompt = json.dumps(request, separators=(",", ":"), ensure_ascii=True)

    global _AGENT
    if _AGENT is None:
        _AGENT = build_agent()

    result = await _AGENT.ainvoke(
        {"messages": [HumanMessage(content=prompt)]},
        config={"configurable": {"thread_id": request["thread_id"]}},
    )
    final = result["messages"][-1].content
    files = result.get("files", {})
    return {
        "response": final,
        "artifacts": {
            path: value
            for path, value in files.items()
            if path in {"/treatment.json", "/candidate.json"}
        },
        "models": {
            "planner": PLANNER_MODEL,
            "generator": GENERATOR_MODEL,
            "orchestrator": ORCHESTRATOR_MODEL,
        },
    }


def validate_candidate_json(raw: str) -> CandidatePatch:
    """Public seam used by tests and the future Rust response adapter."""

    try:
        return CandidatePatch.model_validate_json(raw).enforce_budget()
    except (ValidationError, ValueError) as error:
        raise ValueError(f"invalid candidate: {error}") from error
