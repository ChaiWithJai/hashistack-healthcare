//! Verified escalation ladder (#4, decision 0001): routing emerges from
//! **verification, not prediction**.
//!
//! The supervisor climbs the fixed ladder rules → local → frontier. After
//! each attempt it runs a deterministic verifier (gates preflight
//! before/after on a cloned app record, plus cheap structural checks) and
//! the verdict decides: accept, or record the failed attempt and climb. No
//! tier is trusted; every tier is checked the same way — so a wrong, empty,
//! or unreachable model can only ever cost an attempt, never corrupt an app.
//!
//! The ladder is the outer authority; the pack's signed `routing` policy
//! expresses consent within it: which tier tries each action FIRST, and
//! which failure classes may spend frontier tokens (`escalate_on`). When
//! every model tier fails, the rules floor still lands the doctor's edit.
//!
//! Every action is a Waypoint-style Operation row (steering §4) upserted
//! RUNNING **before** any driver runs: a Running/Escalated row with no
//! terminal status IS the record of an interrupted action.

use std::env;

use crate::agent::{AgentDriver, AgentReply, HttpModelDriver, RuleBasedDriver, ScaffoldStep};
use crate::gates::{self, GateReport};
use crate::packs::{EscalationReason, PackManifest, RoutingPolicy, RoutingTier};
use crate::state::{now_unix, AppRecord, AttemptRecord, OpKind, OpStatus, Operation, Platform};

/// The verifier's binary call on one attempt's output.
enum Verdict {
    Accept,
    Reject(String),
}

/// Top-of-ladder failure: every tier's output was rejected. The app record
/// is untouched; the operation row holds the full attempt history.
#[derive(Debug)]
pub struct LadderFailure {
    pub op_id: String,
    pub reason: String,
}

/// The fixed ladder the agent supervisor climbs, cheapest tier first.
pub struct EscalationLadder {
    tiers: Vec<(String, Box<dyn AgentDriver>)>,
}

impl EscalationLadder {
    /// Rules always; `LOCAL_MODEL_URL` adds the in-VPC local tier;
    /// `FRONTIER_MODEL_URL` adds the frontier tier. No env vars → rules-only,
    /// and behavior is byte-identical to the pre-ladder platform (the rules
    /// driver always verifies today).
    pub fn from_env() -> Self {
        let var = |name: &str| env::var(name).ok().filter(|v| !v.trim().is_empty());
        let mut tiers: Vec<(String, Box<dyn AgentDriver>)> =
            vec![("rules".to_string(), Box::new(RuleBasedDriver))];
        if let Some(url) = var("LOCAL_MODEL_URL") {
            tiers.push(("local".to_string(), Box::new(HttpModelDriver::local(url))));
        }
        if let Some(url) = var("FRONTIER_MODEL_URL") {
            tiers.push((
                "frontier".to_string(),
                Box::new(HttpModelDriver::frontier(url)),
            ));
        }
        Self { tiers }
    }

    /// Custom ladders — how tests compose mock tiers. Order low → high.
    pub fn with_tiers(tiers: Vec<(String, Box<dyn AgentDriver>)>) -> Self {
        Self { tiers }
    }

    /// The climb for one action: start at the policy's first tier (degraded
    /// to the nearest configured tier below when its endpoint is not
    /// configured), climb upward, and finish on the rules floor so the
    /// doctor's edit still lands when every model tier is down.
    fn climb(&self, first: RoutingTier) -> (Vec<usize>, String) {
        let rank = |name: &str| match name {
            "rules" => 0u8,
            "frontier" => 2,
            _ => 1,
        };
        let want = format!("{first}");
        let start = self
            .tiers
            .iter()
            .position(|(n, _)| *n == want)
            .map(|i| (i, String::new()))
            .unwrap_or_else(|| {
                // Nearest configured tier below the requested one, else the
                // lowest configured tier. from_env always configures rules.
                let i = self
                    .tiers
                    .iter()
                    .enumerate()
                    .filter(|(_, (n, _))| rank(n) < rank(&want))
                    .max_by_key(|(_, (n, _))| rank(n))
                    .or_else(|| {
                        self.tiers
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, (n, _))| rank(n))
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let resolved = self.tiers.get(i).map(|(n, _)| n.as_str()).unwrap_or("none");
                (
                    i,
                    format!(" ({want} unconfigured — resolved to {resolved})"),
                )
            });
        let (start, note) = start;
        let mut seq: Vec<usize> = (start..self.tiers.len()).collect();
        if let Some(floor) = self.tiers.iter().position(|(n, _)| n == "rules") {
            if floor < start {
                seq.push(floor); // degradation floor, not an escalation
            }
        }
        (seq, note)
    }

    /// Run the scaffold action up the ladder. The operation row is upserted
    /// Running before any driver runs; the accepted step list is returned and
    /// the caller builds the app record from it.
    pub fn run_scaffold(
        &self,
        plat: &mut Platform,
        app_id: &str,
        pack: &PackManifest,
        prompt: &str,
    ) -> Result<Vec<ScaffoldStep>, LadderFailure> {
        let mut op = self.open_operation(plat, app_id, OpKind::Scaffold, pack);
        let op_id = op.op_id.clone();
        self.run_climb(plat, &mut op, pack, 1, |driver| {
            let steps = driver.scaffold(pack, prompt);
            if steps.is_empty() {
                (steps, Verdict::Reject("empty-scaffold".to_string()))
            } else {
                (steps, Verdict::Accept)
            }
        })
        .map_err(|reason| LadderFailure { op_id, reason })
    }

    /// Run one iterate action up the ladder. Each tier edits a **clone** of
    /// the app record; only a verified edit is committed back, so a full-
    /// ladder failure leaves the app untouched by construction.
    pub fn run_iterate(
        &self,
        plat: &mut Platform,
        app_id: &str,
        instruction: &str,
        pack: &PackManifest,
    ) -> Result<(AgentReply, String), LadderFailure> {
        let required = &pack.gates;
        let mut op = self.open_operation(plat, app_id, OpKind::Iterate, pack);
        let op_id = op.op_id.clone();

        let Some(before) = plat.apps.get(app_id).cloned() else {
            self.settle_exhausted(plat, &mut op);
            return Err(LadderFailure {
                op_id,
                reason: "app not found".to_string(),
            });
        };
        let report_before = gates::preflight(&before, required);

        let (reply, candidate) = self
            .run_climb(plat, &mut op, pack, before.current_version + 1, |driver| {
                let mut candidate = before.clone();
                let reply = driver.iterate(&mut candidate, instruction, required);
                let verdict = verify_iterate(&before, &report_before, &candidate, required);
                ((reply, candidate), verdict)
            })
            .map_err(|reason| LadderFailure {
                op_id: op_id.clone(),
                reason,
            })?;
        plat.apps.insert(app_id.to_string(), candidate);
        Ok((reply, op_id))
    }

    /// One climb: run `attempt` per rung (skipping an unconsented frontier),
    /// settle every verdict, stop at the first accepted output. Errs with
    /// the last rejection reason when the whole ladder is exhausted.
    fn run_climb<T>(
        &self,
        plat: &mut Platform,
        op: &mut Operation,
        pack: &PackManifest,
        version: u32,
        mut attempt: impl FnMut(&dyn AgentDriver) -> (T, Verdict),
    ) -> Result<T, String> {
        let policy = pack.routing_policy();
        let source = pack.routing_source();
        let (seq, _) = self.climb(policy.first_tier(op.kind));
        let mut last_reason = String::from("no tiers configured");
        let total = seq.len();
        for (pos, idx) in seq.into_iter().enumerate() {
            let (tier, driver) = &self.tiers[idx];
            if self.frontier_withheld(plat, op, tier, pos, &policy, &source, &last_reason) {
                continue;
            }
            let started = now_unix();
            let (value, verdict) = attempt(driver.as_ref());
            match verdict {
                Verdict::Accept => {
                    self.settle_accept(plat, op, tier, started, version);
                    return Ok(value);
                }
                Verdict::Reject(reason) => {
                    self.settle_reject(plat, op, tier, started, version, &reason, pos + 1 == total);
                    last_reason = reason;
                }
            }
        }
        self.settle_exhausted(plat, op);
        Err(last_reason)
    }

    /// Escalation consent (decision 0001, grafted from 4b): a climb INTO the
    /// frontier rung spends tokens only for failure classes the pack named
    /// in `escalate_on`. First-rung frontier (policy said so) needs no
    /// consent; the rules floor below is degradation, never escalation.
    #[allow(clippy::too_many_arguments)]
    fn frontier_withheld(
        &self,
        plat: &mut Platform,
        op: &Operation,
        tier: &str,
        pos: usize,
        policy: &RoutingPolicy,
        source: &str,
        last_reason: &str,
    ) -> bool {
        if pos == 0 || tier != "frontier" {
            return false;
        }
        let class = reason_class(last_reason);
        if policy.escalate_on.contains(&class) {
            return false;
        }
        plat.audit.record(
            "agent",
            "agent.routed",
            format!("per {source}: {class} not in escalate_on — frontier withheld"),
            Some(&op.app_id),
        );
        true
    }

    /// Upsert the operation row Running BEFORE any driver work — the
    /// Waypoint invariant. A crash after this point leaves visible evidence.
    /// The routing decision is audited here, citing the policy that made it.
    fn open_operation(
        &self,
        plat: &mut Platform,
        app_id: &str,
        kind: OpKind,
        pack: &PackManifest,
    ) -> Operation {
        let op = Operation {
            op_id: plat.mint_id("op"),
            app_id: app_id.to_string(),
            kind,
            status: OpStatus::Running,
            attempts: Vec::new(),
            started_at: now_unix(),
            finished_at: None,
        };
        plat.upsert_operation(op.clone());
        let first = pack.routing_policy().first_tier(kind);
        let (_, note) = self.climb(first);
        plat.audit.record(
            "agent",
            "agent.routed",
            format!(
                "per {}: {}→{first}{note}",
                pack.routing_source(),
                kind.as_str()
            ),
            Some(app_id),
        );
        op
    }

    fn settle_accept(
        &self,
        plat: &mut Platform,
        op: &mut Operation,
        tier: &str,
        started: u64,
        version: u32,
    ) {
        op.attempts.push(AttemptRecord {
            tier: tier.to_string(),
            started_at: started,
            finished_at: now_unix(),
            verdict: "accepted".to_string(),
            reason: None,
        });
        op.status = OpStatus::Success;
        op.finished_at = Some(now_unix());
        plat.upsert_operation(op.clone());
        plat.audit.record(
            "agent",
            "agent.attempt",
            format!(
                "op {} {} v{version} tier={tier} verdict=accepted → applied",
                op.op_id,
                op.kind.as_str()
            ),
            Some(&op.app_id),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn settle_reject(
        &self,
        plat: &mut Platform,
        op: &mut Operation,
        tier: &str,
        started: u64,
        version: u32,
        reason: &str,
        top_of_ladder: bool,
    ) {
        op.attempts.push(AttemptRecord {
            tier: tier.to_string(),
            started_at: started,
            finished_at: now_unix(),
            verdict: "rejected".to_string(),
            reason: Some(reason.to_string()),
        });
        op.status = if top_of_ladder {
            OpStatus::Failed
        } else {
            OpStatus::Escalated
        };
        if top_of_ladder {
            op.finished_at = Some(now_unix());
        }
        plat.upsert_operation(op.clone());
        plat.audit.record(
            "agent",
            "agent.attempt",
            format!(
                "op {} {} v{version} tier={tier} verdict={reason} → {}",
                op.op_id,
                op.kind.as_str(),
                if top_of_ladder { "failed" } else { "climbing" }
            ),
            Some(&op.app_id),
        );
    }

    /// Terminalize an operation that never produced an accepted attempt
    /// (exhausted ladder, withheld frontier, empty ladder, missing app) so
    /// no row leaks a false Running or Escalated.
    fn settle_exhausted(&self, plat: &mut Platform, op: &mut Operation) {
        if op.status != OpStatus::Success {
            op.status = OpStatus::Failed;
        }
        op.finished_at = Some(now_unix());
        plat.upsert_operation(op.clone());
    }
}

/// Map a verifier rejection onto the policy's consent classes: gate
/// regressions are their own class; everything else (empty, unparseable,
/// unknown-control, unreachable) is an invalid edit.
fn reason_class(reason: &str) -> EscalationReason {
    if reason.starts_with("gate-regression") {
        EscalationReason::GateRegression
    } else {
        EscalationReason::InvalidEdit
    }
}

/// The verifier: deterministic, model-free, and identical for every tier.
/// Checks run cheapest-first; the gate preflight is the load-bearing one —
/// an edit may never turn a green check red.
fn verify_iterate(
    before: &AppRecord,
    report_before: &GateReport,
    candidate: &AppRecord,
    required: &[String],
) -> Verdict {
    // 1. Newly wired controls must name real gates — a model cannot invent
    //    a safeguard the registry doesn't know.
    let unknown: Vec<String> = candidate
        .controls
        .difference(&before.controls)
        .filter(|c| !gates::known_gate(c))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Verdict::Reject(format!("unknown-control({})", unknown.join(", ")));
    }

    // 2. No gate regression: every check satisfied before the edit (passing
    //    or a labeled stub — anything that wasn't blocking) must still be
    //    satisfied after it. preflight evaluates `required` in order, so
    //    the reports zip positionally.
    let report_after = gates::preflight(candidate, required);
    let lost: Vec<String> = report_before
        .results
        .iter()
        .zip(report_after.results.iter())
        .filter(|(b, a)| b.outcome.satisfied() && !a.outcome.satisfied())
        .map(|(_, a)| a.id.clone())
        .collect();
    if !lost.is_empty() {
        return Verdict::Reject(format!("gate-regression({} lost)", lost.join(", ")));
    }

    // 3. The edit must actually do something — an unreachable endpoint or
    //    unparseable reply degrades to a no-op, which lands here.
    if candidate.features.len() == before.features.len() && candidate.controls == before.controls {
        return Verdict::Reject("empty-edit".to_string());
    }

    Verdict::Accept
}
