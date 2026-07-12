# Proof pull request

## User problem

Name the doctor or practice role. Describe the task they can complete after
this change. Link a transcript source, user session, or written research note.

## Scope

Name the one problem this PR solves. List work that is intentionally left out.
If the PR changes more than one subsystem, explain why it cannot be split.

## Architecture

- [ ] The user workflow stays the same or becomes simpler.
- [ ] Validation happens before side effects.
- [ ] External work is idempotent or has a tested compensation path.
- [ ] Desired state and observed state are separate where they can differ.
- [ ] A decision record covers any contested or hard to reverse choice.

## Evidence

- [ ] Contract test proves the user task.
- [ ] Failure test proves the unsafe path stays blocked.
- [ ] Concurrency or restart test covers durable state changes.
- [ ] Browser or clean room evidence covers the final artifact when applicable.
- [ ] All review comments are resolved in public.
- [ ] Required CI checks pass on the current head.

## End user footprint

Report the change against `origin/main`.

- Changed files:
- Added and removed lines:
- New dependencies:
- New services or processes:
- New configuration fields or environment variables:
- Exported artifact size:
- Cold start time and memory, when runtime code changes:

Explain why a smaller design would not meet the same user need.

## Safety and rollback

Name the main failure mode. Explain how an operator can detect it. Describe the
rollback or compensation path and link its test.

## Documentation

- [ ] README and user instructions match current behavior.
- [ ] Runbook covers setup, failure, and rollback.
- [ ] Known limitations are visible next to capability claims.
- [ ] Generated evidence can be reproduced by a command in this PR.

## Verification

```bash
scripts/merge-gate.sh
scripts/merge-gate.sh --full
```
