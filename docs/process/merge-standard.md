# Merge standard

## Purpose

We merge a change when it improves a doctor or practice workflow and keeps the
system small enough to operate. Passing tests are required. They are one part
of the decision.

This standard applies patterns recorded in `docs/hashicorp-steering.md`:

- Keep the user workflow stable while implementations can change.
- Validate a plan before side effects begin.
- Store desired state separately from observed state.
- Write operation state before starting work.
- Refuse an operation when its audit record cannot be written.
- Keep plugin and pack configuration typed and centrally validated.
- Record hard to reverse choices in a decision record.

## Required gates

A PR is ready only when every gate passes.

### User gate

The PR names one end user and one task. It links transcript evidence or a user
session. A contract or browser test proves the task.

### Scope gate

The PR solves one problem. A PR that changes more than 20 files or 1,500 lines
needs a written split analysis. Generated evidence is counted separately, but
the command that creates it must be in the PR.

### Architecture gate

Pure validation happens before external work. The code is safe to retry. When
external work can partly succeed, the PR includes compensation and tests it.
Concurrent requests cannot restore an old whole record over newer work.

### Safety gate

A labeled stub is not a passing production control. Synthetic demo exceptions
must use separate data, credentials, scheduling, and labels. Secrets never
appear in logs or exports. Tenant boundaries fail closed.

### Evidence gate

Formatting, lint, unit tests, contract tests, and the pressure test pass on the
current commit. Runtime or artifact changes include a job test. State changes
include restart or concurrency evidence. The PR links the output.

### Documentation gate

The README, runbook, limitation notes, and decision records agree with the
code. A reviewer can reproduce generated evidence. The issue bar is checked
against current behavior before the issue is closed.

### Review gate

At least one person other than the author reviews the PR. Blocking comments are
resolved in public. A disputed point has a written decision. The final commit
has green checks after the last change.

### Footprint gate

The PR reports new dependencies, processes, settings, and services. A new
moving part needs a user or safety benefit that existing code cannot provide.

Default budgets for an exported starter are:

| Measure | Default budget |
|---|---:|
| Application source | 75 KB excluding lock files and fixtures |
| Export bundle | 250 KB excluding screenshots and build output |
| External browser dependencies | 0 |
| Required runtime processes | 1 application process |
| Startup time on a developer laptop | 1 second after build |
| Resident memory at idle | 64 MB |

Going over a budget is allowed when the PR shows why the user task requires it.

## Change organization

Use four reviewed groups for this body of work:

1. Platform contracts. This group covers state, audit, identity, secrets, and
   deployment. Each side effect lifecycle must stand on its own.
2. Owned packs. This group covers compact runnable packs, fixtures, and pack
   contracts. It should not change control plane semantics.
3. Evaluation. This group covers scenario runners, artifact checks, and
   reproducible evidence. Generated scorecards come from this group.
4. Clinician experience. This group covers the user interface, the end to end
   journey, transcript based validation, and documentation.

Do not merge a stack by pointing each PR at an unmerged feature branch forever.
Merge the first reviewed group to `main`, then rebase and retarget the next
group. Preserve decision commits. Squash generated evidence churn and formatting
fixes when they do not carry a decision.

## Issue closure

An issue closes only when its written bar is proven on the merged default
branch. A later PR may provide part of the work without closing the issue. The
issue comment should link the test, run, and limitation that justify closure.
