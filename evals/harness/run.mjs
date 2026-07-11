#!/usr/bin/env node
// Platform eval harness — two nested layers, scored per scenario.
//
// Layer 1 (job-to-be-done): can a doctor/CHP vibe-code this scenario? Boots
// a fresh in-memory control plane per scenario and drives the whole
// describe → iterate → gate → (fix) → review → promote → export workflow
// over real HTTP, scoring the workflow contract (gate shape, false-pass
// guard, attestation, audit reconstructability, ejection completeness).
//
// Layer 2 (artifact): is what got produced actually good? For packs that
// ship a runnable scaffold (post-op-monitor today, #5), unpacks the ejected
// bundle, builds and RUNS the ejected app, and drives it with Playwright:
// does it render, does it do the clinical job (a pain-9 check-in routes a
// flag; a pain-2 does not), and does it keep its honesty markers (the
// encryption stub is labeled, never claimed). Unconverted packs score
// "no-artifact (#5 pending)" — visible in the scorecard, never skipped
// silently.
//
// Identity (#10): the harness is an ordinary API client — every request
// carries a Phase 0 dev bearer token from staging/identities.hcl, and two
// auth scenarios assert the tenancy wall (dr-park vs dr-osei's tenant) and
// the staff role denial (ms-rivera may not co-sign a release).
//
// Exit code is nonzero only on harness errors or a failing check in a
// scenario marked must_pass.
//
// Invoked by scripts/evals.sh (which builds the control plane and pins
// CARGO_TARGET_DIR); see evals/README.md for the scenario schema.

import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const SCENARIO_DIR = path.join(ROOT, 'evals', 'scenarios');
const EVALS_DIR = path.join(ROOT, '.evals');
const SHOTS_DIR = path.join(EVALS_DIR, 'screenshots');
const LOGS_DIR = path.join(EVALS_DIR, 'logs');
const BUNDLES_DIR = path.join(EVALS_DIR, 'bundles');
const DOCS_DIR = path.join(ROOT, 'docs', 'evals');
const DOCS_SHOTS_DIR = path.join(DOCS_DIR, 'screenshots');
const CONTROL_PLANE_BIN = path.join(
  process.env.CARGO_TARGET_DIR || path.join(ROOT, 'target'), 'debug', 'rust-proof-service');
const EJECT_TARGET_DIR = process.env.EVALS_EJECT_TARGET_DIR || path.join(EVALS_DIR, 'target');
const CP_PORT_BASE = 39200; // control-plane instances (siblings use 39000/39100)
const APP_PORT_BASE = 39300; // ejected apps

// ---------- playwright resolution (preinstalled path first, then normal) ----------

async function loadPlaywright() {
  const candidates = [
    process.env.EVALS_PLAYWRIGHT_MODULE,
    '/opt/node22/lib/node_modules/playwright/index.mjs',
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return import(pathToFileURL(candidate).href);
  }
  return import('playwright'); // CI: npm-installed next to the repo root
}

// ---------- tiny process + http helpers ----------

const children = new Set();
function reap() {
  for (const child of children) {
    try { child.kill('SIGKILL'); } catch { /* already gone */ }
  }
}
process.on('exit', reap);
process.on('SIGINT', () => { reap(); process.exit(130); });
process.on('SIGTERM', () => { reap(); process.exit(143); });

function spawnServer(bin, env, logPath, cwd) {
  const log = fs.openSync(logPath, 'w');
  const child = spawn(bin, [], {
    cwd, env: { ...process.env, ...env }, stdio: ['ignore', log, log],
  });
  child.on('exit', () => fs.closeSync(log));
  children.add(child);
  return child;
}

async function waitHealthy(base, child, logPath, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`server exited early (code ${child.exitCode}) — see ${logPath}`);
    }
    try {
      const res = await fetch(`${base}/health`);
      if (res.ok) return;
    } catch { /* not up yet */ }
    await sleep(100);
  }
  throw new Error(`server at ${base} never became healthy — see ${logPath}`);
}

function stopServer(child) {
  try { child.kill('SIGTERM'); } catch { /* already gone */ }
  children.delete(child);
}

// Identity (#10): the harness is an ordinary API client, so it
// authenticates like one — the Phase 0 dev bearer tokens declared in
// staging/identities.hcl (embedded as the compile-time dev registry).
// Every request carries a token; nothing rides the dev fallback anymore.
const TOKENS = {
  'dr-osei': 'dev-token-osei', // clinician, tenant meridian (the default persona)
  'dr-park': 'dev-token-park', // clinician, tenant lakeside
  'ms-rivera': 'dev-token-rivera', // staff, tenant meridian
};
const DEFAULT_PRINCIPAL = 'dr-osei';

async function api(base, method, route, body, principal = DEFAULT_PRINCIPAL) {
  const headers = {};
  if (principal !== null) {
    // `principal: null` sends no Authorization header (used to assert the
    // strict/fallback boundary); an unknown name is sent verbatim as a bad
    // token so 401 paths are testable.
    headers.authorization = `Bearer ${TOKENS[principal] ?? principal}`;
  }
  if (body !== undefined) headers['content-type'] = 'application/json';
  const res = await fetch(`${base}${route}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* non-JSON (audit JSONL export) */ }
  return { status: res.status, json, text };
}

// ---------- scoring ----------

function makeChecklist() {
  const checks = [];
  return {
    checks,
    add(name, pass, evidence, opts = {}) {
      checks.push({ name, status: pass ? 'pass' : 'fail', evidence, ...opts });
      return pass;
    },
    note(name, status, evidence) {
      checks.push({ name, status, evidence });
    },
  };
}

const setEq = (a, b) => a.length === b.length && [...a].sort().join() === [...b].sort().join();

// ---------- layer 1: the job to be done ----------

async function runWorkflow(scenario, base, score) {
  const wf = scenario.workflow;
  const steps = [];
  const fail = (why) => { score.add('workflow_completed', false, why); return null; };

  // describe → generate
  const created = await api(base, 'POST', '/api/apps',
    { prompt: scenario.prompt, pack: scenario.pack, name: scenario.app_name });
  if (created.status !== 200) return fail(`create returned ${created.status}: ${created.text.slice(0, 200)}`);
  const appId = created.json.app.id;
  steps.push(`created ${appId} (sandbox, ${created.json.app.data_source.kind})`);
  if (created.json.app.stage !== 'sandbox') return fail(`new app stage is ${created.json.app.stage}, not sandbox`);
  if (created.json.app.data_source.kind !== 'synthetic') return fail('new app is not on synthetic data');

  // gate: the initial report must have the pack's exact shape
  const gate0 = await api(base, 'GET', `/api/apps/${appId}/gate`);
  if (gate0.status !== 200) return fail(`gate returned ${gate0.status}`);
  const report0 = gate0.json.report;
  const failing0 = report0.results.filter((r) => r.status === 'fail');
  const fixable0 = failing0.filter((r) => r.fixable).map((r) => r.id);
  const shapeOk =
    report0.total === wf.gate_total &&
    report0.stubbed === wf.stubbed &&
    report0.green === false &&
    setEq(failing0.map((r) => r.id), wf.initially_failing) &&
    setEq(fixable0, wf.fixable_failing);
  score.add('gate_shape_matches', shapeOk,
    `initial report ${report0.passed}/${report0.total} (${report0.stubbed} stubbed), ` +
    `failing: [${failing0.map((r) => r.id).join(', ')}], fixable: [${fixable0.join(', ')}] — ` +
    `expected total ${wf.gate_total}, failing [${wf.initially_failing.join(', ')}]`);

  // false-pass guard: promotion refused while any check fails
  if (wf.assert_false_pass_guard) {
    const refused = await api(base, 'POST', `/api/apps/${appId}/promote`, { cosigner: wf.cosigner });
    const names = refused.status === 409 && /deploy locked/.test(refused.json?.error ?? '');
    score.add('false_pass_guard', names,
      `promote-while-failing returned ${refused.status}: ${(refused.json?.error ?? refused.text).slice(0, 160)}`);
    if (!names) return fail('platform promoted (or mis-refused) an app with failing checks');
  }

  // iterate: conversational edits; expected wired controls per instruction
  for (const step of scenario.iterate ?? []) {
    const it = await api(base, 'POST', `/api/apps/${appId}/iterate`, { instruction: step.instruction });
    if (it.status !== 200) return fail(`iterate ${JSON.stringify(step.instruction)} returned ${it.status}: ${it.text.slice(0, 200)}`);
    const wired = it.json.reply.wired_controls ?? [];
    if (!setEq(wired, step.expect_wired)) {
      return fail(`iterate ${JSON.stringify(step.instruction)} wired [${wired.join(', ')}], expected [${step.expect_wired.join(', ')}]`);
    }
    steps.push(`iterated (${step.vocabulary}-vocabulary): wired [${wired.join(', ') || 'nothing'}], feature added`);
  }

  // restore (edge): roll the record back to a checkpoint, keep wired controls up to it
  if (scenario.restore_to_version) {
    const before = await api(base, 'GET', `/api/apps/${appId}`);
    const restored = await api(base, 'POST', `/api/apps/${appId}/restore`, { version: scenario.restore_to_version });
    if (restored.status !== 200) return fail(`restore returned ${restored.status}`);
    const droppedFeatures = before.json.features.length - restored.json.features.length;
    const ok = restored.json.current_version === scenario.restore_to_version && droppedFeatures > 0;
    score.add('restore_checkpoint', ok,
      `restored v${restored.json.current_version} (asked v${scenario.restore_to_version}); ` +
      `dropped ${droppedFeatures} feature(s); controls now [${restored.json.controls.join(', ')}]`);
    if (!ok) return fail('restore did not rebuild the expected checkpoint');
    steps.push(`restored checkpoint v${scenario.restore_to_version}`);
  }

  // fix it for me: one-click wiring for the gates the conversation left failing
  for (const gateId of wf.fix_gates ?? []) {
    const fixed = await api(base, 'POST', `/api/apps/${appId}/gate/${gateId}/fix`, {});
    if (fixed.status !== 200) return fail(`fix ${gateId} returned ${fixed.status}: ${fixed.text.slice(0, 200)}`);
    steps.push(`fixed ${gateId} (one click)`);
  }

  // platform review, where the pack demands human-review
  if (wf.review) {
    const review = await api(base, 'POST', `/api/apps/${appId}/review`, {});
    if (review.status !== 200) return fail(`review returned ${review.status}`);
    if (!review.json.report.green) return fail(`review found the app not ready: ${review.json.reviewer_note}`);
    steps.push('platform review attached (co-sign card)');
  }

  // gate must now be green, then promote with a co-signature
  const gate1 = await api(base, 'GET', `/api/apps/${appId}/gate`);
  if (!gate1.json.report.green) {
    const still = gate1.json.report.results.filter((r) => r.status === 'fail').map((r) => r.id);
    return fail(`gate not green before promote — still failing: [${still.join(', ')}]`);
  }
  const promoted = await api(base, 'POST', `/api/apps/${appId}/promote`, { cosigner: wf.cosigner });
  if (promoted.status !== 200) return fail(`promote returned ${promoted.status}: ${promoted.text.slice(0, 200)}`);
  if (promoted.json.app.stage !== 'live') return fail('promoted app is not live');
  steps.push(`promoted to live (gate ${promoted.json.report.passed}/${promoted.json.report.total} green, co-signed ${wf.cosigner})`);

  // attestation: co-signature bound to the frozen gate report
  const att = promoted.json.app.attestation;
  score.add('attestation_present',
    !!att && att.cosigner === wf.cosigner && !!att.gate_summary && !!att.report,
    att ? `attestation by ${att.cosigner}, gate summary ${att.gate_summary}, report frozen: ${!!att.report}`
        : 'no attestation on the promoted record');

  // audit: the story must be reconstructable from the append-only stream
  const audit = await api(base, 'GET', `/api/apps/${appId}/audit`);
  const actions = new Set((audit.json?.events ?? []).map((e) => e.action));
  const wanted = ['app.created', 'app.promoted', 'gate.passed'];
  if ((scenario.iterate ?? []).length > 0) wanted.push('app.iterated');
  if (scenario.restore_to_version) wanted.push('app.restored');
  const missing = wanted.filter((a) => !actions.has(a));
  score.add('audit_reconstructable', missing.length === 0,
    missing.length === 0
      ? `${audit.json.events.length} events; create/iterate/promote all present`
      : `audit stream is missing: [${missing.join(', ')}]`);

  // agent tier per operation — recorded, not judged (the honest rules floor)
  const ops = await api(base, 'GET', `/api/apps/${appId}/operations`);
  const tiers = {};
  for (const op of ops.json?.operations ?? []) {
    for (const attempt of op.attempts) {
      const key = `${op.kind}:${attempt.tier}`;
      tiers[key] = (tiers[key] ?? 0) + 1;
    }
  }
  score.note('agent_tiers', 'recorded', Object.entries(tiers).map(([k, v]) => `${k}×${v}`).join(', ') || 'none');

  // eject: the owned bundle, complete, carrying the doctor's actual prompt
  const exported = await api(base, 'GET', `/api/apps/${appId}/export`);
  if (exported.status !== 200) return fail(`export returned ${exported.status}`);
  const files = exported.json.files;
  const missingCore = wf.eject_core_files.filter((f) => !(f in files));
  const scaffoldOk = !wf.eject_scaffold_source ||
    ('app/src/main.rs' in files && 'app/Cargo.toml' in files && 'synthetic/post-op-demo.json' in files);
  const promptCarried = (files['README.md'] ?? '').includes(scenario.prompt);
  score.add('eject_bundle_complete',
    missingCore.length === 0 && scaffoldOk && promptCarried,
    `${Object.keys(files).length} files; core missing: [${missingCore.join(', ')}]; ` +
    `scaffold source: ${wf.eject_scaffold_source ? (scaffoldOk ? 'present' : 'MISSING') : 'n/a (#5 pending)'}; ` +
    `README carries the doctor's prompt: ${promptCarried}`);
  steps.push(`ejected ${Object.keys(files).length}-file bundle`);

  score.add('workflow_completed', true, steps.join(' → '));
  return { appId, bundle: exported.json, tiers };
}

// The duplicate-names edge wraps the standard workflow with a same-name twin.
async function runDuplicateNames(scenario, base, score) {
  const mk = () => api(base, 'POST', '/api/apps',
    { prompt: scenario.prompt, pack: scenario.pack, name: scenario.app_name });
  const first = await mk();
  const second = await mk();
  const okCreated = first.status === 200 && second.status === 200;
  const idA = first.json?.app?.id, idB = second.json?.app?.id;
  const distinct = okCreated && idA !== idB;
  const list = await api(base, 'GET', '/api/apps');
  const both = (list.json?.apps ?? []).filter((a) => a.name === scenario.app_name).length === 2;
  score.add('duplicate_names_distinct_ids', distinct && both,
    `same name twice → ids ${JSON.stringify(idA)} and ${JSON.stringify(idB)}; both listed: ${both}`);
  if (!distinct || !both) { score.add('workflow_completed', false, 'duplicate-name creation failed'); return null; }

  // Drive the SECOND app through the full workflow; the first must not move.
  const wf = scenario.workflow;
  const refused = await api(base, 'POST', `/api/apps/${idB}/promote`, { cosigner: wf.cosigner });
  score.add('false_pass_guard', refused.status === 409 && /deploy locked/.test(refused.json?.error ?? ''),
    `promote-while-failing on the twin returned ${refused.status}`);
  const gate0 = await api(base, 'GET', `/api/apps/${idB}/gate`);
  const failing0 = gate0.json.report.results.filter((r) => r.status === 'fail');
  score.add('gate_shape_matches',
    gate0.json.report.total === wf.gate_total && setEq(failing0.map((r) => r.id), wf.initially_failing),
    `twin gate ${gate0.json.report.passed}/${gate0.json.report.total}, failing [${failing0.map((r) => r.id).join(', ')}]`);
  for (const gateId of wf.fix_gates) await api(base, 'POST', `/api/apps/${idB}/gate/${gateId}/fix`, {});
  const promoted = await api(base, 'POST', `/api/apps/${idB}/promote`, { cosigner: wf.cosigner });
  const att = promoted.json?.app?.attestation;
  score.add('attestation_present', promoted.status === 200 && !!att && !!att.report,
    promoted.status === 200 ? `twin promoted, attestation by ${att?.cosigner}` : `promote returned ${promoted.status}`);
  const firstAgain = await api(base, 'GET', `/api/apps/${idA}`);
  const untouched = firstAgain.json?.stage === 'sandbox' && !firstAgain.json?.attestation;
  score.add('twin_isolation', untouched,
    `original app after twin promotion: stage=${firstAgain.json?.stage}, attestation=${!!firstAgain.json?.attestation}`);
  const audit = await api(base, 'GET', `/api/apps/${idB}/audit`);
  const actions = new Set((audit.json?.events ?? []).map((e) => e.action));
  score.add('audit_reconstructable', actions.has('app.created') && actions.has('app.promoted'),
    `${audit.json?.events?.length ?? 0} events on the twin's stream`);
  const exported = await api(base, 'GET', `/api/apps/${idB}/export`);
  const files = exported.json?.files ?? {};
  const missingCore = wf.eject_core_files.filter((f) => !(f in files));
  score.add('eject_bundle_complete',
    exported.status === 200 && missingCore.length === 0 && (files['README.md'] ?? '').includes(scenario.prompt),
    `${Object.keys(files).length} files; core missing: [${missingCore.join(', ')}]`);
  score.add('workflow_completed', promoted.status === 200 && untouched,
    `created twins ${idA}/${idB} → promoted ${idB} only → ejected`);
  const ops = await api(base, 'GET', `/api/apps/${idB}/operations`);
  const tiers = {};
  for (const op of ops.json?.operations ?? []) {
    for (const attempt of op.attempts) {
      const key = `${op.kind}:${attempt.tier}`;
      tiers[key] = (tiers[key] ?? 0) + 1;
    }
  }
  score.note('agent_tiers', 'recorded', Object.entries(tiers).map(([k, v]) => `${k}×${v}`).join(', ') || 'none');
  return { appId: idB, bundle: exported.json, tiers };
}

// Identity (#10): tenancy is a wall, not a list filter. dr-park builds in
// lakeside; a meridian app id must answer 404 (existence undisclosed), the
// denial must land on the OWNING tenant's audit stream (review-log P12),
// and a present-but-wrong token is 401 even in dev mode (P9).
async function runTwoTenant(scenario, base, score) {
  // dr-osei (meridian) creates first — the app whose existence must stay
  // invisible across the tenant boundary.
  const meridian = await api(base, 'POST', '/api/apps',
    { prompt: 'home BP log for my clinic', pack: 'hypertension-tracker', name: 'meridian bp log' },
    'dr-osei');
  // dr-park (lakeside) creates their own; the tenant comes from the
  // principal, never the request body.
  const lakeside = await api(base, 'POST', '/api/apps',
    { prompt: scenario.prompt, pack: scenario.pack, name: scenario.app_name }, 'dr-park');
  const created = meridian.status === 200 && lakeside.status === 200;
  score.add('tenant_derived_from_principal',
    created && meridian.json?.app?.tenant === 'meridian' && lakeside.json?.app?.tenant === 'lakeside',
    `dr-osei's app tenant=${meridian.json?.app?.tenant}, dr-park's app tenant=${lakeside.json?.app?.tenant} — both derived from the bearer token, no tenant field sent`);
  if (!created) { score.add('workflow_completed', false, `creates returned ${meridian.status}/${lakeside.status}`); return null; }
  const meridianId = meridian.json.app.id;

  // cross-tenant fetch: 404 — never 403, existence is not disclosed
  const cross = await api(base, 'GET', `/api/apps/${meridianId}`, undefined, 'dr-park');
  score.add('cross_tenant_fetch_404', cross.status === 404,
    `dr-park (lakeside) fetching meridian app ${meridianId} → ${cross.status} (must be 404: existence undisclosed, exactly like a nonexistent id)`);

  // the list is the same boundary: dr-park sees lakeside and nothing else
  const parkList = await api(base, 'GET', '/api/apps', undefined, 'dr-park');
  const parkApps = parkList.json?.apps ?? [];
  score.add('list_tenant_scoped',
    parkApps.length === 1 && parkApps.every((a) => a.tenant === 'lakeside'),
    `dr-park's list: ${parkApps.length} app(s), tenants [${[...new Set(parkApps.map((a) => a.tenant))].join(', ')}]`);

  // P12: the denial lands on the owning tenant's stream, actor = the denied principal
  const audit = await api(base, 'GET', `/api/apps/${meridianId}/audit`, undefined, 'dr-osei');
  const denial = (audit.json?.events ?? []).find(
    (e) => e.action === 'auth.cross_tenant_denied' && e.actor === 'dr-park');
  score.add('denial_audited_on_owning_stream', !!denial,
    denial ? `meridian's stream carries auth.cross_tenant_denied by dr-park: ${denial.detail.slice(0, 100)}`
           : 'auth.cross_tenant_denied by dr-park missing from the owning tenant\'s stream');

  // a present-but-wrong token is 401 even in dev mode (P9)
  const badToken = await api(base, 'GET', '/api/apps', undefined, 'not-a-real-token');
  score.add('wrong_token_401', badToken.status === 401,
    `garbage bearer token → ${badToken.status} (dev fallback applies only to a MISSING header, never a wrong token)`);

  score.add('workflow_completed', cross.status === 404 && !!denial,
    `two tenants created → cross-tenant fetch denied as 404 → denial audited on meridian's stream → wrong token 401`);
  return null;
}

// Identity (#10): releasing to real patients is a clinical act. Staff share
// the tenant view, but promotion answers 403 + auth.role_denied — and the
// same green gate then promotes for the clinician, proving the 403 was the
// role, not the app.
async function runStaffDenial(scenario, base, score) {
  const wf = scenario.workflow;
  const created = await api(base, 'POST', '/api/apps',
    { prompt: scenario.prompt, pack: scenario.pack, name: scenario.app_name }, 'dr-osei');
  if (created.status !== 200) { score.add('workflow_completed', false, `create returned ${created.status}`); return null; }
  const appId = created.json.app.id;

  // staff share the practice: in-tenant read is 200, not a 404 game
  const staffView = await api(base, 'GET', `/api/apps/${appId}`, undefined, 'ms-rivera');
  score.add('staff_can_view_in_tenant', staffView.status === 200,
    `ms-rivera (staff, same tenant) reading the app → ${staffView.status}`);

  // the clinician drives the gate green
  for (const gateId of wf.fix_gates ?? []) {
    await api(base, 'POST', `/api/apps/${appId}/gate/${gateId}/fix`, {}, 'dr-osei');
  }
  const gate = await api(base, 'GET', `/api/apps/${appId}/gate`, undefined, 'dr-osei');
  if (!gate.json?.report?.green) { score.add('workflow_completed', false, 'gate never went green'); return null; }

  // ...but staff cannot co-sign a release: 403, in-tenant so existence is known
  const denied = await api(base, 'POST', `/api/apps/${appId}/promote`, {}, 'ms-rivera');
  score.add('staff_promote_403',
    denied.status === 403 && /may not/.test(denied.json?.error ?? ''),
    `ms-rivera promoting a GREEN app → ${denied.status}: ${(denied.json?.error ?? denied.text).slice(0, 120)}`);

  // the denial is audited with the denied principal as actor
  const audit = await api(base, 'GET', `/api/apps/${appId}/audit`, undefined, 'dr-osei');
  const denial = (audit.json?.events ?? []).find(
    (e) => e.action === 'auth.role_denied' && e.actor === 'ms-rivera');
  score.add('denial_audited', !!denial,
    denial ? `auth.role_denied by ms-rivera on the app stream: ${denial.detail.slice(0, 100)}`
           : 'auth.role_denied by ms-rivera missing from the app stream');

  // the same green gate promotes for the clinician — and the attestation
  // binds the authenticated principal id + the frozen report digest (#10)
  const promoted = await api(base, 'POST', `/api/apps/${appId}/promote`,
    { cosigner: wf.cosigner }, 'dr-osei');
  const att = promoted.json?.app?.attestation;
  score.add('clinician_cosign_binds_principal',
    promoted.status === 200 && att?.principal === 'dr-osei' && !!att?.report_digest,
    promoted.status === 200
      ? `promoted by dr-osei: attestation principal=${att?.principal}, report digest ${String(att?.report_digest).slice(0, 16)}…`
      : `clinician promote returned ${promoted.status}: ${(promoted.json?.error ?? promoted.text).slice(0, 120)}`);

  score.add('workflow_completed', denied.status === 403 && promoted.status === 200,
    'created → gate green → staff promote 403 (audited) → clinician promote 200 with principal-bound attestation');
  return null;
}

// Refusal scenarios: the platform SHOULD refuse with a written reason
// (GOAL.md bar 7, RFC 0001). There is no refusal surface yet (#12), so
// these run, fail honestly, and land in the scorecard as known gaps.
async function runRefusal(scenario, base, score) {
  const created = await api(base, 'POST', '/api/apps',
    { prompt: scenario.prompt, pack: scenario.pack });
  const refused = created.status !== 200;
  const reasoned = refused && /scope|refus/i.test(created.json?.error ?? '');
  score.add('refused_with_reason', refused && reasoned,
    refused
      ? `refused with: ${(created.json?.error ?? created.text).slice(0, 160)}`
      : `NOT refused — the platform scaffolded ${JSON.stringify(created.json?.app?.id)} from an out-of-scope prompt ` +
        `(RFC 0001 use case ${scenario.rfc_use_case}); no refusal surface exists yet (#12)`,
    { expected_fail: true });
  return null;
}

// ---------- layer 2: the artifact ----------

function unpackBundle(bundle, dir) {
  fs.rmSync(dir, { recursive: true, force: true });
  for (const [rel, content] of Object.entries(bundle.files)) {
    const dest = path.join(dir, rel);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.writeFileSync(dest, content);
  }
}

let builtAppHash = null;
function buildEjectedApp(bundleDir, scenarioId) {
  const hash = createHash('sha256')
    .update(fs.readFileSync(path.join(bundleDir, 'app', 'src', 'main.rs')))
    .update(fs.readFileSync(path.join(bundleDir, 'app', 'Cargo.toml')))
    .digest('hex');
  const bin = path.join(EJECT_TARGET_DIR, 'debug', 'app');
  if (builtAppHash === hash && fs.existsSync(bin)) return bin; // compiled once, shared target dir
  console.log(`    [layer2] building the ejected app (${scenarioId}) …`);
  execFileSync('cargo', ['build', '--quiet'], {
    cwd: path.join(bundleDir, 'app'),
    env: { ...process.env, CARGO_TARGET_DIR: EJECT_TARGET_DIR },
    stdio: ['ignore', 'inherit', 'inherit'],
  });
  builtAppHash = hash;
  return bin;
}

async function runArtifactChecks(scenario, bundle, index, browser, score) {
  const bundleDir = path.join(BUNDLES_DIR, scenario.id);
  unpackBundle(bundle, bundleDir);
  const bin = buildEjectedApp(bundleDir, scenario.id);

  const port = APP_PORT_BASE + index;
  const base = `http://127.0.0.1:${port}`;
  const logPath = path.join(LOGS_DIR, `${scenario.id}-app.jsonl`);
  const child = spawnServer(bin, {
    APP_BIND: `127.0.0.1:${port}`,
    SYNTHETIC_DATA: path.join(bundleDir, 'synthetic', 'post-op-demo.json'),
  }, logPath, path.join(bundleDir, 'app'));

  const shot = (name) => path.join(SHOTS_DIR, `${scenario.id}-${name}.png`);
  const context = await browser.newContext({ viewport: { width: 960, height: 720 } });
  try {
    await waitHealthy(base, child, logPath);
    const page = await context.newPage();
    // Keep the check hermetic: the scaffold's Google-Fonts links must never
    // make the harness depend on an external network.
    await page.route(/fonts\.(googleapis|gstatic)\.com/, (r) => r.abort());

    // (a) renders: form fields, SYNTHETIC banner, the sketchy-kit skin
    await page.goto(`${base}/`, { waitUntil: 'domcontentloaded' });
    const painField = await page.locator('input[name="pain"]').count();
    const woundField = await page.locator('select[name="wound"]').count();
    const patientField = await page.locator('select[name="patient_id"]').count();
    const banner = await page.getByText('synthetic data only').count();
    const sketchyRadius = painField
      ? await page.locator('.sk').first().evaluate((el) => getComputedStyle(el).borderRadius)
      : '';
    const html = await page.content();
    const sketchyKit = sketchyRadius.includes('225px') && html.includes('Patrick Hand');
    await page.screenshot({ path: shot('home'), fullPage: true });
    score.add('artifact_renders',
      painField === 1 && woundField === 1 && patientField === 1 && banner > 0 && sketchyKit,
      `pain/wound/patient fields: ${painField}/${woundField}/${patientField}; ` +
      `SYNTHETIC banner hits: ${banner}; sketchy-kit border-radius on .sk: ${JSON.stringify(sketchyRadius)}; ` +
      `screenshot: ${path.relative(ROOT, shot('home'))}`);

    // (b) does the job: pain 9 → flag routed; pain 2 → no flag
    await page.selectOption('select[name="patient_id"]', 'pt-001');
    await page.fill('input[name="pain"]', '9');
    await page.selectOption('select[name="wound"]', 'clean');
    await page.fill('input[name="note"]', 'much worse since last night');
    await page.screenshot({ path: shot('form'), fullPage: true });
    await page.click('button[type="submit"]');
    const flagConfirmed = await page.getByText('flag routed to the practice inbox').count();
    await page.screenshot({ path: shot('flag'), fullPage: true });
    await page.goto(`${base}/`, { waitUntil: 'domcontentloaded' });
    const inboxShowsFlag = (await page.content()).includes('pain 9/10');

    await page.selectOption('select[name="patient_id"]', 'pt-003');
    await page.fill('input[name="pain"]', '2');
    await page.selectOption('select[name="wound"]', 'clean');
    await page.click('button[type="submit"]');
    const noEscalation = await page.getByText('no escalation').count();
    await page.goto(`${base}/`, { waitUntil: 'domcontentloaded' });
    const inboxStillOne = ((await page.content()).match(/pain \d+\/10 at or over threshold/g) ?? []).length === 1;

    await sleep(150); // let stdout flush
    const auditLines = fs.readFileSync(logPath, 'utf8').split('\n')
      .filter((l) => l.includes('"control":"audit-log"') && l.includes('"path":"/checkin"'));
    score.add('artifact_does_the_job',
      flagConfirmed > 0 && inboxShowsFlag && noEscalation > 0 && inboxStillOne && auditLines.length === 2,
      `pain-9 flag routed: ${flagConfirmed > 0} (inbox shows it: ${inboxShowsFlag}); ` +
      `pain-2 not escalated: ${noEscalation > 0} (inbox count still 1: ${inboxStillOne}); ` +
      `stdout audit JSONL recorded ${auditLines.length}/2 check-ins; ` +
      `screenshots: ${path.relative(ROOT, shot('form'))}, ${path.relative(ROOT, shot('flag'))}`);

    // (c) honesty markers: the encryption stub is labeled, exactly where the
    // pack's gate report says `stubbed`
    const homeHtml = (await page.content());
    const stubLabeled = homeHtml.includes('encryption at rest is a labeled TODO')
      && homeHtml.includes('hipaa-core placeholder');
    score.add('artifact_honesty_markers', stubLabeled,
      `encryption-stub label visible on the running page: ${homeHtml.includes('encryption at rest is a labeled TODO')}; ` +
      `audit-placeholder label in the footer: ${homeHtml.includes('hipaa-core placeholder')}`);
  } catch (err) {
    score.add('artifact_renders', false, `artifact layer crashed: ${err.message}`);
  } finally {
    await context.close();
    stopServer(child);
  }
}

// ---------- scorecard rendering ----------

function aggregate(results) {
  const rate = (checks) => {
    const applicable = checks.filter((c) => c.status === 'pass' || c.status === 'fail');
    const passed = applicable.filter((c) => c.status === 'pass').length;
    const expectedFail = applicable.filter((c) => c.status === 'fail' && c.expected_fail).length;
    return { applicable: applicable.length, passed, failed: applicable.length - passed, expected_fail: expectedFail };
  };
  const layer1 = rate(results.flatMap((r) => r.layer1.checks));
  const layer2 = rate(results.flatMap((r) => r.layer2.checks));
  const perPack = {};
  for (const r of results) {
    // Refusals and identity scenarios aggregate under their own rows: they
    // judge platform surfaces, not the pack a doctor might have grabbed.
    const key = r.category === 'refusal' ? 'refusals (RFC 9/10/15/21)'
      : r.category === 'auth' ? 'identity/tenancy (#10)' : r.pack;
    perPack[key] ??= { scenarios: 0, layer1: { passed: 0, applicable: 0 }, layer2: { passed: 0, applicable: 0, no_artifact: 0 } };
    const p = perPack[key];
    p.scenarios += 1;
    for (const c of r.layer1.checks) {
      if (c.status === 'pass' || c.status === 'fail') { p.layer1.applicable += 1; if (c.status === 'pass') p.layer1.passed += 1; }
    }
    for (const c of r.layer2.checks) {
      if (c.status === 'pass' || c.status === 'fail') { p.layer2.applicable += 1; if (c.status === 'pass') p.layer2.passed += 1; }
      if (c.status === 'no-artifact') p.layer2.no_artifact += 1;
    }
  }
  return { layer1, layer2, perPack };
}

function pct(passed, applicable) {
  return applicable === 0 ? '—' : `${Math.round((passed / applicable) * 100)}%`;
}

function renderMarkdown(results, agg, meta) {
  const lines = [];
  lines.push('# Platform eval scorecard — the portable baseline');
  lines.push('');
  lines.push(`Generated ${meta.generated_at} at commit \`${meta.commit}\` by \`scripts/evals.sh\` (${meta.runtime_seconds}s). Machine-readable twin: [scorecard.json](scorecard.json).`);
  lines.push('');
  lines.push('## What this measures');
  lines.push('');
  lines.push('Two nested layers over a realistic sampling of how doctors and community');
  lines.push('health workers actually phrase things (per pack: precise physician,');
  lines.push('colloquial physician, CHP home-visit idiom, terse/typo\'d — plus refusal');
  lines.push('and edge scenarios):');
  lines.push('');
  lines.push('- **Layer 1 — the job to be done.** Can this persona vibe-code the tool?');
  lines.push('  A fresh in-memory control plane per scenario, driven over real HTTP');
  lines.push('  through describe → iterate → gate → fix → review → promote → eject,');
  lines.push('  scoring the workflow contract (gate shape, false-pass guard,');
  lines.push('  attestation, audit reconstructability, bundle completeness).');
  lines.push('- **Layer 2 — the artifact.** Is what got produced actually good? The');
  lines.push('  ejected bundle is unpacked, **built, and run**, and Playwright drives');
  lines.push('  the running ejected app: it must render the clinical form (with the');
  lines.push('  SYNTHETIC banner and the sketchy-kit skin), do the clinical job (a');
  lines.push('  pain-9 check-in routes a flag to the practice inbox and the stdout');
  lines.push('  audit log; a pain-2 does not), and keep its honesty markers (the');
  lines.push('  encryption stub labeled, never claimed). Only post-op-monitor ships a');
  lines.push('  runnable scaffold today, so the other four packs score **no-artifact');
  lines.push('  (#5 pending)** — visible, never silently skipped.');
  lines.push('');
  lines.push('## Summary');
  lines.push('');
  lines.push('| | scenarios | layer 1 checks | layer 2 checks |');
  lines.push('|---|---|---|---|');
  lines.push(`| **all** | ${results.length} | ${agg.layer1.passed}/${agg.layer1.applicable} passed (${pct(agg.layer1.passed, agg.layer1.applicable)}; ${agg.layer1.expected_fail} expected-fail) | ${agg.layer2.passed}/${agg.layer2.applicable} passed (${pct(agg.layer2.passed, agg.layer2.applicable)}) |`);
  for (const [pack, p] of Object.entries(agg.perPack)) {
    const l2 = p.layer2.applicable > 0
      ? `${p.layer2.passed}/${p.layer2.applicable} (${pct(p.layer2.passed, p.layer2.applicable)})`
      : (p.layer2.no_artifact > 0 ? 'no-artifact (#5 pending)' : 'n/a');
    lines.push(`| ${pack} | ${p.scenarios} | ${p.layer1.passed}/${p.layer1.applicable} (${pct(p.layer1.passed, p.layer1.applicable)}) | ${l2} |`);
  }
  lines.push('');
  lines.push('## Per-scenario results');
  lines.push('');
  lines.push('| scenario | persona | pack | layer 1 | layer 2 | agent tier |');
  lines.push('|---|---|---|---|---|---|');
  for (const r of results) {
    const l1 = r.layer1.checks.filter((c) => c.status === 'pass' || c.status === 'fail');
    const l1Passed = l1.filter((c) => c.status === 'pass').length;
    const l1Cell = r.category === 'refusal'
      ? (l1Passed === l1.length ? 'refused ✓' : '**not refused — known gap (#12)**')
      : `${l1Passed}/${l1.length}${l1Passed === l1.length ? '' : ' ⚠'}`;
    const l2 = r.layer2.checks;
    const l2Applicable = l2.filter((c) => c.status === 'pass' || c.status === 'fail');
    let l2Cell = 'n/a';
    if (l2Applicable.length > 0) {
      const p = l2Applicable.filter((c) => c.status === 'pass').length;
      l2Cell = `${p}/${l2Applicable.length}${p === l2Applicable.length ? '' : ' ⚠'}`;
    } else if (l2.some((c) => c.status === 'no-artifact')) {
      l2Cell = 'no-artifact (#5)';
    }
    const tiers = Object.keys(r.agent_tiers ?? {}).map((k) => k.split(':')[1]);
    const tierCell = tiers.length ? [...new Set(tiers)].join(', ') : '—';
    lines.push(`| ${r.id} | ${r.persona} | ${r.pack} | ${l1Cell} | ${l2Cell} | ${tierCell} |`);
  }
  lines.push('');
  lines.push('## Evidence: the running ejected app');
  lines.push('');
  lines.push('Screenshots below are of the **ejected** post-op-monitor app — unpacked');
  lines.push('from the export bundle, compiled, and driven by the harness (the full');
  lines.push('set for every post-op scenario lands in `.evals/screenshots/`, gitignored):');
  lines.push('');
  lines.push('| the form renders | a pain-9 check-in filled | the flag routed |');
  lines.push('|---|---|---|');
  lines.push('| ![home](screenshots/home.png) | ![form](screenshots/form.png) | ![flag](screenshots/flag.png) |');
  lines.push('');
  lines.push('## Known gaps (expected failures — this is a regression baseline, not a trophy)');
  lines.push('');
  for (const gap of meta.known_gaps) lines.push(`- ${gap}`);
  lines.push('');
  lines.push('## Failing checks in full');
  lines.push('');
  const failures = results.flatMap((r) =>
    r.layer1.checks.concat(r.layer2.checks)
      .filter((c) => c.status === 'fail')
      .map((c) => ({ id: r.id, ...c })));
  if (failures.length === 0) {
    lines.push('None.');
  } else {
    lines.push('| scenario | check | expected fail? | evidence |');
    lines.push('|---|---|---|---|');
    for (const f of failures) {
      lines.push(`| ${f.id} | ${f.name} | ${f.expected_fail ? 'yes — known gap' : '**NO — regression**'} | ${f.evidence.replace(/\|/g, '\\|')} |`);
    }
  }
  lines.push('');
  lines.push('## Methodology');
  lines.push('');
  lines.push(`${results.length} scenarios (${meta.counts.pack} pack workflows across 5 packs × 4+ personas, ` +
    `${meta.counts.refusal} refusals from RFC 0001's out-of-scope list, ${meta.counts.edge} edges: duplicate names, restore-then-promote, ` +
    `${meta.counts.auth} identity scenarios: two-tenant isolation, staff role denial), ` +
    'each against a freshly booted in-memory control plane on its own port — no shared state, no mocked HTTP. ' +
    'Every request authenticates with the Phase 0 dev bearer tokens from staging/identities.hcl (#10) — nothing rides the dev fallback. ' +
    'The agent ladder runs at its rules floor (no model endpoints configured), and the scorecard records that tier per operation: ' +
    'this baseline measures what the platform honestly does today, not what a frontier model might add. ' +
    'Layer 2 builds the ejected bundle with a worktree-local shared CARGO_TARGET_DIR (compiles once), boots each ejected app on its own port, ' +
    'and drives it with Playwright/Chromium; external font hosts are blocked so the check is hermetic. ' +
    'Expected-fail checks (refusals) exit zero; any failing check in a must_pass scenario exits nonzero. ' +
    'Add a scenario: drop a JSON file in evals/scenarios/ (schema in evals/README.md) and re-run `scripts/evals.sh`.');
  lines.push('');
  return lines.join('\n');
}

// ---------- main ----------

async function main() {
  const t0 = Date.now();
  for (const dir of [EVALS_DIR, SHOTS_DIR, LOGS_DIR, BUNDLES_DIR, DOCS_DIR, DOCS_SHOTS_DIR]) {
    fs.mkdirSync(dir, { recursive: true });
  }
  if (!fs.existsSync(CONTROL_PLANE_BIN)) {
    throw new Error(`control plane binary missing at ${CONTROL_PLANE_BIN} — run scripts/evals.sh (it builds first)`);
  }

  const scenarios = fs.readdirSync(SCENARIO_DIR).filter((f) => f.endsWith('.json')).sort()
    .map((f) => JSON.parse(fs.readFileSync(path.join(SCENARIO_DIR, f), 'utf8')));
  console.log(`== eval harness: ${scenarios.length} scenarios, two layers\n`);

  const { chromium } = await loadPlaywright();
  const browser = await chromium.launch();

  const results = [];
  let index = 0;
  for (const scenario of scenarios) {
    const i = index++;
    console.log(`-- [${String(i + 1).padStart(2)}/${scenarios.length}] ${scenario.id} (${scenario.category}, ${scenario.pack})`);
    const layer1 = makeChecklist();
    const layer2 = makeChecklist();

    // layer 1 against a fresh control plane on its own port
    const port = CP_PORT_BASE + i;
    const base = `http://127.0.0.1:${port}`;
    const cpLog = path.join(LOGS_DIR, `${scenario.id}-control-plane.log`);
    const cp = spawnServer(CONTROL_PLANE_BIN, { APP_BIND: `127.0.0.1:${port}`, CONTROL_DB_URL: '' }, cpLog);
    let outcome = null;
    try {
      await waitHealthy(base, cp, cpLog);
      if (scenario.category === 'refusal') outcome = await runRefusal(scenario, base, layer1);
      else if (scenario.category === 'auth' && scenario.auth_flow === 'two-tenant') outcome = await runTwoTenant(scenario, base, layer1);
      else if (scenario.category === 'auth' && scenario.auth_flow === 'staff-denial') outcome = await runStaffDenial(scenario, base, layer1);
      else if (scenario.edge === 'duplicate-names') outcome = await runDuplicateNames(scenario, base, layer1);
      else outcome = await runWorkflow(scenario, base, layer1);
    } finally {
      stopServer(cp);
    }

    // layer 2 over the ejected artifact
    if (scenario.category === 'refusal') {
      layer2.note('artifact', 'n/a', 'refusal scenario — nothing may be scaffolded, so there is no artifact to judge');
    } else if (scenario.category === 'auth') {
      layer2.note('artifact', 'n/a', 'identity scenario — the artifact is judged by the pack workflows, not re-judged here');
    } else if (scenario.artifact?.expected === 'playwright') {
      if (outcome?.bundle) {
        await runArtifactChecks(scenario, outcome.bundle, i, browser, layer2);
      } else {
        layer2.add('artifact_renders', false, 'layer 1 produced no bundle to judge');
      }
    } else {
      layer2.note('artifact', 'no-artifact', `${scenario.artifact?.reason ?? 'no runnable scaffold'} — scored visibly, not skipped`);
    }

    for (const c of [...layer1.checks, ...layer2.checks]) {
      const mark = c.status === 'pass' ? 'ok  ' : c.status === 'fail' ? (c.expected_fail ? 'GAP ' : 'FAIL') : 'note';
      console.log(`    ${mark} ${c.name}: ${c.evidence.slice(0, 140)}`);
    }
    results.push({
      id: scenario.id, category: scenario.category, persona: scenario.persona,
      pack: scenario.pack, must_pass: scenario.must_pass,
      layer1: { checks: layer1.checks }, layer2: { checks: layer2.checks },
      agent_tiers: outcome?.tiers ?? {},
    });
  }
  await browser.close();

  // promote the three evidence screenshots into the committed docs
  const best = 'post-op-01-precise-physician';
  for (const name of ['home', 'form', 'flag']) {
    const src = path.join(SHOTS_DIR, `${best}-${name}.png`);
    if (fs.existsSync(src)) fs.copyFileSync(src, path.join(DOCS_SHOTS_DIR, `${name}.png`));
  }

  const agg = aggregate(results);
  const counts = { pack: 0, refusal: 0, edge: 0, auth: 0 };
  for (const r of results) counts[r.category] = (counts[r.category] ?? 0) + 1;
  const tiersSeen = new Set(results.flatMap((r) => Object.keys(r.agent_tiers).map((k) => k.split(':')[1])));
  const commit = execFileSync('git', ['rev-parse', '--short', 'HEAD'], { cwd: ROOT }).toString().trim();
  const meta = {
    generated_at: new Date().toISOString(),
    commit,
    runtime_seconds: Math.round((Date.now() - t0) / 1000),
    counts,
    known_gaps: [
      `**No refusal surface (#12).** All ${counts.refusal} out-of-scope scenarios (RFC 0001 use cases 9, 10, 15, 21) were scaffolded instead of refused-with-a-reason — GOAL.md bar 7 is unmet and these checks fail by design until the refusal surface lands.`,
      '**Four packs have no runnable scaffold (#5).** hypertension-tracker, patient-intake, compliance-checklist, and insurance-verification eject the honest placeholder runtime, so their artifact layer scores no-artifact — the post-op-monitor pattern needs porting.',
      `**Agent tier floor: ${[...tiersSeen].join(', ') || 'rules'}.** No model endpoints were configured, so every scaffold/iterate ran on the deterministic rules driver — this baseline is the honest floor, not a measure of model-tier quality (decision 0002 keeps CI/sandbox model-free by design).`,
    ],
  };

  const scorecard = { ...meta, totals: { scenarios: results.length, layer1: agg.layer1, layer2: agg.layer2 }, per_pack: agg.perPack, scenarios: results };
  fs.writeFileSync(path.join(DOCS_DIR, 'scorecard.json'), JSON.stringify(scorecard, null, 2) + '\n');
  fs.writeFileSync(path.join(DOCS_DIR, 'scorecard.md'), renderMarkdown(results, agg, meta));
  console.log(`\n== scorecard written: docs/evals/scorecard.md + scorecard.json (${meta.runtime_seconds}s)`);
  console.log(`   layer 1: ${agg.layer1.passed}/${agg.layer1.applicable} (${agg.layer1.expected_fail} expected-fail) — layer 2: ${agg.layer2.passed}/${agg.layer2.applicable}`);

  // exit contract: nonzero only for harness errors or must_pass regressions
  const regressions = results.filter((r) => r.must_pass).flatMap((r) =>
    r.layer1.checks.concat(r.layer2.checks)
      .filter((c) => c.status === 'fail' && !c.expected_fail)
      .map((c) => `${r.id}/${c.name}`));
  if (regressions.length > 0) {
    console.error(`\n== REGRESSIONS in must_pass scenarios:\n   ${regressions.join('\n   ')}`);
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(`harness error: ${err.stack ?? err}`);
  process.exit(2);
});
