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
from pydantic import BaseModel, Field, ValidationError, model_validator


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

    @model_validator(mode="after")
    def validate_treatment_ids(self) -> "TreatmentPlan":
        ids = [item.id for item in self.treatments]
        if len(set(ids)) != len(ids):
            raise ValueError("treatment ids must be unique")
        if self.recommended_treatment_id not in ids:
            raise ValueError("recommended treatment must exist")
        return self


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


def _state_file_text(value: Any, path: str) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, dict):
        content = value.get("content")
        encoding = value.get("encoding", "utf-8")
        if isinstance(content, str) and encoding == "utf-8":
            return content
    raise ValueError(f"{path} is not a UTF-8 state file")


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


GENERATION_PROMPT = """You are the Practice Studio source-workspace generator.

The Rust control plane has already shown treatments to the user and supplied
their explicit selected_treatment_id. Follow the Open SWE composed-harness
pattern for this generation action only:
1. Write a short todo list.
2. Call generate_workspace_patch exactly once with the selected treatment,
   current workspace summary, pack constraints, and acceptance checks.
3. Save only the candidate patch to /candidate.json using the thread-local
   StateBackend.
4. Return a short completion note.

Never claim that code compiled or passed a browser check. The Rust control
plane performs those checks after this worker returns. Never request secrets,
patient data, GitHub access, shell access, or deployment access."""


_GENERATION_AGENT = None


def build_generation_agent():
    return create_deep_agent(
        model=_model(ORCHESTRATOR_MODEL),
        system_prompt=GENERATION_PROMPT,
        tools=[generate_workspace_patch],
        backend=StateBackend,
    )


def normalize_request(data: dict[str, Any]) -> dict[str, Any]:
    allowed = {
        "schema_version",
        "action",
        "thread_id",
        "task",
        "pack",
        "workspace_summary",
        "selected_treatment_id",
    }
    unknown = sorted(set(data) - allowed)
    if unknown:
        raise ValueError(f"unknown request fields: {', '.join(unknown)}")
    schema_version = data.get("schema_version")
    if type(schema_version) is not int or schema_version != 1:
        raise ValueError("schema_version must be 1")
    action = data.get("action")
    if action not in {"plan", "generate"}:
        raise ValueError("action must be plan or generate")
    task = str(data.get("task", "")).strip()
    pack = str(data.get("pack", "")).strip()
    if not task or not pack:
        raise ValueError("task and pack are required")
    selected_treatment_id = str(data.get("selected_treatment_id", "")).strip()
    if action == "generate" and not selected_treatment_id:
        raise ValueError("selected_treatment_id is required for generate")
    if action == "plan" and selected_treatment_id:
        raise ValueError("selected_treatment_id is not accepted for plan")
    return {
        "schema_version": 1,
        "action": action,
        "thread_id": str(data.get("thread_id", "")).strip() or "ephemeral",
        "task": task,
        "pack": pack,
        "workspace_summary": str(data.get("workspace_summary", "")).strip(),
        "selected_treatment_id": selected_treatment_id,
    }


async def run_action(data: dict[str, Any]) -> dict[str, Any]:
    """Run one strict worker action and return its canonical version-1 shape."""

    request = normalize_request(data)
    prompt = json.dumps(request, separators=(",", ":"), ensure_ascii=True)

    if request["action"] == "plan":
        treatment_raw = plan_treatments.invoke(prompt)
        treatment_plan = validate_treatment_json(treatment_raw)
        return {
            "schema_version": 1,
            "treatment_plan": treatment_plan.model_dump(),
        }

    global _GENERATION_AGENT
    if _GENERATION_AGENT is None:
        _GENERATION_AGENT = build_generation_agent()

    result = await _GENERATION_AGENT.ainvoke(
        {"messages": [HumanMessage(content=prompt)]},
        config={"configurable": {"thread_id": request["thread_id"]}},
    )
    files = result.get("files", {})
    candidate_raw = _state_file_text(files.get("/candidate.json"), "/candidate.json")
    candidate_patch = validate_candidate_json(candidate_raw)
    return {
        "schema_version": 1,
        "candidate_patch": candidate_patch.model_dump(),
    }


@entrypoint
async def main(data: dict[str, Any], context: dict[str, Any]):
    """DigitalOcean ADK entrypoint for one strict version-1 action."""

    del context
    return await run_action(data)


def validate_treatment_json(raw: str) -> TreatmentPlan:
    """Parse and canonicalize a planner response before it crosses the boundary."""

    try:
        return TreatmentPlan.model_validate_json(raw)
    except ValidationError as error:
        raise ValueError(f"invalid treatment plan: {error}") from error


def validate_candidate_json(raw: str) -> CandidatePatch:
    """Public seam used by tests and the future Rust response adapter."""

    try:
        return CandidatePatch.model_validate_json(raw).enforce_budget()
    except (ValidationError, ValueError) as error:
        raise ValueError(f"invalid candidate: {error}") from error
