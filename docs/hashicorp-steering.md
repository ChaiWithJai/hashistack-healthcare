# HashiCorp source review — patterns steering this platform

We partial-cloned and read the `hashicorp/{nomad,vault,packer,waypoint,boundary}` trees
(~100 recent commits each, plus the extension-point interfaces). This document records
what we're copying and where it lands in our control plane. Referenced from
[RFC 0001, Appendix A](rfc/0001-clinician-platform.md#appendix-a--amendments-from-the-hashicorp-source-review).

---

## 1. Nomad (`plugins/drivers/driver.go`, `nomad/structs/{alloc,deployment}.go`)

**Fat mandatory interface + tiny optional capability interfaces.** The core driver
contract is one interface; optional behaviors are separate interfaces discovered by
type-assertion, with embeddable "not supported" stubs:

```go
type DriverPlugin interface {
    base.BasePlugin
    TaskConfigSchema() (*hclspec.Spec, error)
    Capabilities() (*Capabilities, error)
    Fingerprint(context.Context) (<-chan *Fingerprint, error)
    StartTask(*TaskConfig) (*TaskHandle, *DriverNetwork, error)
    WaitTask(ctx context.Context, taskID string) (<-chan *ExitResult, error)
    StopTask(taskID string, timeout time.Duration, signal string) error
    ...
}
type DriverShutdowner interface { Shutdown(ctx context.Context) error } // optional
func (DriverSignalTaskNotSupported) SignalTask(...) error {
    return fmt.Errorf("SignalTask is not supported by this driver")
}
```

→ **Gate engine:** required trait (`Gate: id/title/evaluate`, in `src/gates.rs` today)
plus optional traits (`GateAutofixer`, `GateReporter`) as they appear; ship
"NotSupported" default impls so plugin authors implement only what they need.

**Plugins self-describe their config schema** (`hclspec.NewObject(...)`), validated
centrally by the agent. → **Pack registry:** packs/gates export a machine-readable
schema; `pack.hcl` validation stays central, schema ownership stays with the plugin.

**Dual-axis status: desired vs observed, explicit terminal sets, promotion as a state.**

```go
AllocDesiredStatusRun|Stop|Evict ; AllocClientStatusPending|Running|Complete|Failed|Lost
DeploymentStatusDescriptionRunningNeedsPromotion = "Deployment is running but requires manual promotion"
```

→ **Deploy service:** apps carry `desired_state` vs `observed_state`; our gate step is
Nomad's canary "requires promotion" made load-bearing.

**Nesting-by-labels declarative config** (`job "x" { group "y" { task "z" {} } }`)
→ **pack.hcl** keeps the `noun "name" { ... }` idiom, platform-owned envelope,
plugin-owned opaque `config {}` blocks.

## 2. Vault (`sdk/logical/`, `audit/`)

**One request-handler interface behind a path router**; every engine receives a uniform,
auditable request envelope (`ID, Operation, Path, Data, ClientToken` — the token
"passed through … after being salted and hashed").

→ **Gate engine / control plane:** route checks through one
`handle(GateRequest) -> GateResponse` shape — the envelope *is* the audit record, so
auditing is uniform and free.

**Audit devices are pluggable backends behind a success-threshold broker:**

```go
// Broker.LogRequest: the operation FAILS unless ≥1 sink durably wrote
if len(status.CompleteSinks()) > 0 { ... return nil }
return fmt.Errorf("error during audit pipeline processing: ...")
```

plus an explicit `IsFallback()` device and a `LogTestMessage` probe before a device is
accepted. → **Audit pipeline invariant: no audit write, no operation.** For HIPAA this
is the difference between "we log" and "logging is load-bearing."

**HMAC-with-salt for sensitive values** (`salt.GetIdentifiedHMAC(data)` with a
`nonHMACDataKeys` allowlist). → PHI fields in audit events are salted-HMAC'd by
default with an explicit plaintext allowlist: searchable, correlatable, not disclosable.

## 3. Packer (`hcl2template/parser.go`, packer-plugin-sdk)

**Typed top-level block grammar with `{type, name}` double labels:**

```hcl
source "amazon-ebs" "ubuntu-1604" { ... }
build { sources = ["source.amazon-ebs.ubuntu-1604"] }
```

→ **pack.hcl v2:** `pack "insurance-verification" "front-desk" {}` (type + instance)
with an `app { packs = [...] }` composing block, so one reviewed pack definition serves
many apps.

**Version-pinned plugin requirements in config:**

```hcl
packer { required_plugins { amazon = { source = "github.com/hashicorp/amazon", version = ">= v4" } } }
```

→ packs carry `required_gates`/`required_packs` with semver constraints resolved by
the registry at plan time — pinning compliance-reviewed pack versions per app.

**Prepare/Run split — "NO side effects should take place in prepare":**

```go
type Builder interface {
    HCL2Speccer
    Prepare(...interface{}) ([]string, []string, error) // pure
    Run(context.Context, Ui, Hook) (Artifact, error)
}
```

→ every gate and generator gets a side-effect-free `validate` phase separate from
`execute`, enabling a dry-run of the full gate plan during preview. The plugin contract
lives in its own SDK crate so third parties never import the control plane.

## 4. Waypoint (`internal/core/`, `internal/config/app.go`)

**Lifecycle as named stages, each binding a plugin via `use`:**

```hcl
app "sinatra" {
  build   { use "pack" {} registry { use "docker" { image = "..." } } }
  deploy  { use "target" { probe_path = "/" } }
  release { use "target" { public = false } }
}
```

This is interface shape, not a supported scheduler instruction. The minimum
lovable runtime uses Docker Compose and does not run Kubernetes or Nomad.

→ **pack.hcl stage blocks** (`generate {}`, `gate {}`, `deploy {}`, `audit {}`) each
containing `use "<plugin>" { ... }`: stage semantics belong to the platform,
implementation to the plugin.

**Uniform "operation" abstraction, upsert-first:**

```go
type operation interface {
    Init(*App) (proto.Message, error)
    Upsert(context.Context, pb.WaypointClient, proto.Message) (proto.Message, error)
    Do(context.Context, hclog.Logger, *App, proto.Message) (interface{}, error)
    Hooks(*App) map[string][]*config.Hook // before/after
}
```

→ every describe→audit step is an operation row upserted RUNNING before work begins:
crash-visible, resumable, auditable by construction. The gate is just an operation
whose `Do` returns pass/fail.

**Release ≠ Deploy, and generations.** `App.Release` routes traffic to an *existing*
deployment; a plugin-computed `Generation{Id}` makes re-deploys mutate one logical
thing. → keep "render + register" separate from "expose to clinicians"; previews update
one logical deployment instead of accreting copies.

**Central URL service with labeled hostnames** (Horizon: vanity FQDN → label-selected
instances). → platform-issued FQDNs mapped by labels (app-id, sequence): instant
preview URLs, traffic-shifting at promote.

## 5. Boundary (`internal/session/`, `internal/target/`, `internal/host/`)

**Database-enforced state machine** — transitions constrained by a table, history
append-only:

```sql
create table session_valid_state( prior_state ..., current_state ..., primary key (prior_state, current_state) );
insert into session_valid_state values
  ('pending','active'), ('pending','terminated'), ('pending','canceling'),
  ('active','canceling'), ('active','terminated'), ('canceling','terminated');
```

→ **App lifecycle:** an `app_valid_state(prior, current)` table plus an append-only
state-history table gives HIPAA auditors provable transition history that application
bugs cannot corrupt. This is the Phase 1 schema for the control DB.

**Operator access as Target + Session.** Target is the policy object
(`GetSessionMaxSeconds()`, `GetSessionConnectionLimit()`, `GetWorkerFilter()`,
`GetEnableSessionRecording()`); Session is the time-boxed grant with per-session
certificate and a `TerminationReason`. → when our staff must touch a clinician's
running app: mint a session against a policy object, never standing access;
`termination_reason` is the audit answer to "why did this access end."

**Catalog/Set/Host three-level abstraction over pluggable providers** (static and
plugin implementations behind one interface, everything project-scoped). → **Pack
registry structure:** Catalog (official / org-private pack sources) → Set (curated
groupings, "cardiology starter kit") → Pack, tenant-scoped from day one.

**Field-level envelope encryption annotations** — paired ct/pt fields plus `KeyId`:

```go
CtTofuToken []byte `gorm:"column:tofu_token" wrapping:"ct,tofu_token"`
TofuToken   []byte `gorm:"-"                 wrapping:"pt,tofu_token"`
```

→ hipaa-core's Rust storage layer adopts ct/pt paired fields + `key_id` (derive macro)
so key rotation and "which key encrypted this row" stay queryable; Boundary even ships
per-domain `rewrapping.go` for rotation.

---

## Commit discipline (synthesized across all five repos)

1. Scope-prefix subjects: `area: summary` — `driver: add optional Init to drivers`,
   `scheduler: reject sticky on a static host volume`, `fix(audit): populate email/name in audit events`.
2. Boundary is strictest (conventional commits: `feat(perms):`, `chore(controller):`);
   Nomad uses bare area prefixes (`client:`, `cli:`, `docker:`). We adopt bare area
   prefixes: `gates:`, `packs:`, `agent:`, `deploy:`, `audit:`, `ui:`, `infra:`, `docs:`.
3. PR number always in the squash-merged subject — every `git log --oneline` line traces to a review.
4. Behavior changes phrased as user-visible facts ("Fixed a bug where task secrets were
   not interpolated into service check `Header`"), not code movements.
5. Changelog-as-files: `.changelog/<PR>.txt` with typed fenced blocks
   (`release-note:improvement` / `bug` / `feature`), merged by tooling at release.
6. Ticket IDs in subjects where an internal tracker exists.
7. Release commits are ritualized and never mixed with feature work
   (`release: main Changelog for 2.0.4`).
8. Backports are tool-driven, labeled early.
9. Reverts keep the full original subject + both PR numbers.
10. Deps/CI noise fenced off with `chore(deps):` / `ci:` so the product history stays readable.
