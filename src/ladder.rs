//! Verified escalation ladder (treatment 4c for #4, investigation 0002 D1):
//! routing emerges from **verification, not prediction**.
//!
//! The supervisor climbs a fixed ladder per agent action — rules → local →
//! frontier. After each attempt it runs a deterministic verifier (gates
//! preflight before/after on a cloned app record, plus cheap structural
//! checks) and the verdict decides: accept, or record the failed attempt and
//! climb. No tier is trusted; every tier is checked the same way — so a
//! wrong, empty, or unreachable model can only ever cost an attempt, never
//! corrupt an app record.
//!
//! Every action is a Waypoint-style Operation row (steering §4) upserted
//! RUNNING **before** the first driver runs: a Running/Escalated row with no
//! terminal status IS the record of an interrupted action.

use std::env;

use crate::agent::{AgentDriver, AgentReply, HttpModelDriver, RuleBasedDriver, ScaffoldStep};
use crate::gates::{self, GateReport, GateStatus};
use crate::packs::PackManifest;
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
        let mut tiers: Vec<(String, Box<dyn AgentDriver>)> =
            vec![("rules".to_string(), Box::new(RuleBasedDriver))];
        if let Ok(url) = env::var("LOCAL_MODEL_URL") {
            tiers.push(("local".to_string(), Box::new(HttpModelDriver::local(url))));
        }
        if let Ok(url) = env::var("FRONTIER_MODEL_URL") {
            tiers.push((
                "frontier".to_string(),
                Box::new(HttpModelDriver::frontier(url)),
            ));
        }
        Self { tiers }
    }

    /// Custom ladders — how tests compose mock tiers, and how a future pack
    /// could shorten or reorder its own ladder.
    pub fn with_tiers(tiers: Vec<(String, Box<dyn AgentDriver>)>) -> Self {
        Self { tiers }
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
        let mut op = self.open_operation(plat, app_id, OpKind::Scaffold);
        let op_id = op.op_id.clone();

        let mut last_reason = String::from("no tiers configured");
        let total = self.tiers.len();
        for (index, (tier, driver)) in self.tiers.iter().enumerate() {
            let started = now_unix();
            let steps = driver.scaffold(pack, prompt);
            let verdict = if steps.is_empty() {
                Verdict::Reject("empty-scaffold".to_string())
            } else {
                Verdict::Accept
            };
            match verdict {
                Verdict::Accept => {
                    self.settle_accept(plat, &mut op, tier, started, 1);
                    return Ok(steps);
                }
                Verdict::Reject(reason) => {
                    self.settle_reject(
                        plat,
                        &mut op,
                        tier,
                        started,
                        1,
                        &reason,
                        index + 1 == total,
                    );
                    last_reason = reason;
                }
            }
        }
        self.settle_exhausted(plat, &mut op);
        Err(LadderFailure {
            op_id,
            reason: last_reason,
        })
    }

    /// Run one iterate action up the ladder. Each tier edits a **clone** of
    /// the app record; only a verified edit is committed back, so a full-
    /// ladder failure leaves the app untouched by construction.
    pub fn run_iterate(
        &self,
        plat: &mut Platform,
        app_id: &str,
        instruction: &str,
        required: &[String],
    ) -> Result<(AgentReply, String), LadderFailure> {
        let mut op = self.open_operation(plat, app_id, OpKind::Iterate);
        let op_id = op.op_id.clone();

        let Some(before) = plat.apps.get(app_id).cloned() else {
            self.settle_exhausted(plat, &mut op);
            return Err(LadderFailure {
                op_id,
                reason: "app not found".to_string(),
            });
        };
        let version = before.current_version + 1;
        let report_before = gates::preflight(&before, required);

        let mut last_reason = String::from("no tiers configured");
        let total = self.tiers.len();
        for (index, (tier, driver)) in self.tiers.iter().enumerate() {
            let started = now_unix();
            let mut candidate = before.clone();
            let reply = driver.iterate(&mut candidate, instruction, required);
            match verify_iterate(&before, &report_before, &candidate, required) {
                Verdict::Accept => {
                    self.settle_accept(plat, &mut op, tier, started, version);
                    plat.apps.insert(app_id.to_string(), candidate);
                    return Ok((reply, op_id));
                }
                Verdict::Reject(reason) => {
                    self.settle_reject(
                        plat,
                        &mut op,
                        tier,
                        started,
                        version,
                        &reason,
                        index + 1 == total,
                    );
                    last_reason = reason;
                }
            }
        }
        self.settle_exhausted(plat, &mut op);
        Err(LadderFailure {
            op_id,
            reason: last_reason,
        })
    }

    /// Upsert the operation row Running BEFORE any driver work — the
    /// Waypoint invariant. A crash after this point leaves visible evidence.
    fn open_operation(&self, plat: &mut Platform, app_id: &str, kind: OpKind) -> Operation {
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
    /// (empty ladder or missing app) so no row leaks a false Running.
    fn settle_exhausted(&self, plat: &mut Platform, op: &mut Operation) {
        if op.status == OpStatus::Running {
            op.status = OpStatus::Failed;
        }
        op.finished_at = Some(now_unix());
        plat.upsert_operation(op.clone());
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

    // 2. No gate regression: every check that passed before the edit must
    //    still pass after it. preflight evaluates `required` in order, so
    //    the reports zip positionally.
    let report_after = gates::preflight(candidate, required);
    let lost: Vec<String> = report_before
        .results
        .iter()
        .zip(report_after.results.iter())
        .filter(|(b, a)| b.outcome == GateStatus::Pass && a.outcome != GateStatus::Pass)
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
