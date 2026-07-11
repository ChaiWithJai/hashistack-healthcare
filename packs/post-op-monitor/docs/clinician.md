# Post-op monitor — what this pack gives you

*Written for the clinician choosing a pack, not for an engineer. This
document ships with the pack and travels into your ejected repository.*

## What it does

You describe the tracker you want ("a post-op recovery tracker for my knee
replacement patients") and this pack gives you a working starting point the
same day:

- **Daily check-ins.** Your patients log pain on a 0–10 scale and how the
  wound looks (clean, redness, swelling, drainage, opening, spreading
  redness), with a free-text note.
- **Wound photos.** Patients can attach a photo to a check-in. In the
  preview sandbox, photos are accepted but *not yet* stored encrypted —
  the app labels this honestly on the upload form, and the encryption
  control must be wired before the promotion gate goes green.
- **Escalation to your inbox.** A check-in at or over pain 7, or a
  concerning wound status, routes a flag to the practice inbox. Flags are
  never something a patient has to hope you noticed on a dashboard.
- **A paper trail by default.** Every request that touches data is written
  to an audit log automatically. You don't turn this on; you can't turn it
  off.
- **Auto-logoff.** An idle session is signed out automatically, the same
  control your EHR uses.

While you build, the app runs entirely on **synthetic patients** — twelve
invented people in `synthetic/post-op-demo.json`, marked as generated and
derived from no real person. Real patients only ever arrive after every
gate below is green and you have co-signed the release.

## What the gates check before real patients

| Gate | In plain language |
|---|---|
| `phi-encryption` | Patient fields and photos are encrypted with keys the app never holds |
| `audit-log` | Every data access leaves a log entry |
| `ai-allowlist` | The app can only call the approved, BAA-covered services |
| `dependency-scan` | The app's building blocks have no known vulnerabilities |
| `auto-logoff` | Idle sessions are signed out |
| `synthetic-only` | The sandbox never saw anything but synthetic data |

This pack also documents an escalation-path check of its own — flags over
threshold must reach the practice inbox — in `gates/README.md`.

## Evidence citations

*Placeholder — to be completed with the clinical advisory review (#3).*
Intended contents: the post-discharge monitoring literature this pack's
defaults are drawn from (pain-threshold escalation, wound-photo triage
turnaround, remote check-in adherence), cited per feature so you can defend
the tool to your colleagues and your compliance officer.

## When you leave

Everything above ejects with you: the app's source code, this document,
your build history, the gate report, and deploy files for four hosting
targets. No part of your tool stays behind on the platform.
