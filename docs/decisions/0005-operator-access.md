# 0005 — Operator access follows Boundary's Target/Session shape

Status: accepted (design record only — **no implementation in Phase 0**)
Date: 2026-07-11
Ticket: #10 (identity/tenancy link; referenced by the TODO(#10) in src/api.rs)
Prior art: Boundary `internal/target` + `internal/session` — docs/hashicorp-steering.md §5

## Context

#10 lands real principals, tenancy 404s, and role capabilities. That
deliberately leaves one hole unplugged rather than plugged badly: platform
staff sometimes need to touch a clinician's *running* app — debug a tenant
deployment, inspect a misbehaving allocation. Today the honest answer is
that they cannot: the staff role has **no cross-tenant read of any kind**
(cross-tenant ids answer 404 like everyone else's), and there is no
break-glass path. This record fixes the shape that access will take so it
is never improvised as standing access.

## Decision: Target + Session, never standing access

Copy Boundary's two-object split:

**Target** — the reviewed *policy object*, one per (tenant, access class),
created ahead of need and signed like a pack:

| field | Boundary source | meaning here |
|---|---|---|
| `session_max_seconds` | `Target.GetSessionMaxSeconds()` | hard ceiling per session (e.g. 30 min for tenant-app debugging) |
| `session_connection_limit` | `Target.GetSessionConnectionLimit()` | how many connections one grant covers |
| `enable_session_recording` | `Target.GetEnableSessionRecording()` | recording flag — ON for anything that can see tenant data |
| `worker_filter` | `Target.GetWorkerFilter()` | which pool/host the session may traverse (prod pool only via the gateway, never the DB directly) |

**Session** — the *time-boxed grant* minted against a Target when a named
staff principal actually needs in:

- bound to: staff principal id + Target + one tenant app + expiry
  (`min(now + session_max_seconds, target ceiling)`);
- state machine `pending → active → canceling → terminated` enforced by the
  control store exactly like `app_valid_state` (#7 — the same
  `session_valid_state` table Boundary uses is already our pattern);
- **`termination_reason` is mandatory** on the terminal row (`expired`,
  `canceled_by_operator`, `canceled_by_tenant`, `connection_limit`) — the
  audit answer to "why did this access end", not just "that it ended";
- every mint/use/termination lands in the audit stream as
  `auth.operator_session_{opened,used,terminated}` with BOTH the staff
  principal and the tenant app on the event, visible in the tenant's own
  app-scoped audit view (the practice sees who was inside their tool);
- the tenant clinician can cancel a live session (that IS
  `canceled_by_tenant`).

## What this rules out

- Standing staff access to tenant data (no role, token, or env var may
  grant it — the capability check in src/identity.rs deliberately has no
  `OperatorRead` today).
- Un-recorded sessions against Targets whose class touches PHI.
- Sessions without an expiry, or terminations without a reason.

## Why not implement it in this link

#10's bar is two-tenant enforcement, and the honest state is "staff cannot
cross tenants at all" — safe-by-absence. A Session implementation is only
honest once there is something real to session INTO (a running allocation —
placement is still virtual in staging, #2/F2). Building the grant machinery
before the thing it grants access to would be exactly the skinned-UI
mistake. When Phase 1 lands real placement, this record is the spec; the
`session_valid_state` table rides the same migration pattern as #7.
