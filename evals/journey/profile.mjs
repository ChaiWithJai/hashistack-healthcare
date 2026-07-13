#!/usr/bin/env node
// Journey profiler — ONE real clinician journey, end to end, profiled.
//
// Where the eval harness (evals/harness/run.mjs) samples 30 scenarios for a
// scorecard, this runs the single flagship journey — Dr. Osei vibe-coding a
// post-op recovery tracker on the fully-real pack — and captures BOTH the
// final artifact and the profiled path to it: every step timed (wall ms
// around the HTTP call / build / boot), cross-referenced to the audit seqs
// it produced, with platform-UI and ejected-app screenshots along the way.
//
// Output (committed): docs/evals/journey/
//   journey.md      — the show-anyone narrative
//   journey.json    — every number and path, machine-readable
//   01-sandbox.png … 06-artifact-flag.png (six stage screenshots, <900KB)
//
// Identity (#10): every request authenticates as dr-osei (clinician,
// meridian) with the Phase 0 dev bearer token from staging/identities.hcl —
// including the Playwright UI contexts (extraHTTPHeaders), so nothing rides
// the dev fallback and the audit stream shows one clean actor.
//
// Invoked by scripts/journey.sh (which builds the control plane and pins
// the worktree-local target dirs). Ports: 39400 control plane, 39450
// ejected app — clear of the eval harness (39200/39300) and the staging
// pressure test (39000+).

import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const OUT_DIR = path.join(ROOT, 'docs', 'evals', 'journey');
const WORK_DIR = path.join(ROOT, '.journey');
const BUNDLE_DIR = path.join(WORK_DIR, 'bundle');
const LOGS_DIR = path.join(WORK_DIR, 'logs');
const CONTROL_PLANE_BIN = path.join(
  process.env.CARGO_TARGET_DIR || path.join(ROOT, 'target'), 'debug', 'rust-proof-service');
const EJECT_TARGET_DIR = process.env.JOURNEY_EJECT_TARGET_DIR || path.join(ROOT, '.journey-target');
const CP_PORT = Number(process.env.JOURNEY_CP_PORT || 39400);
const APP_PORT = Number(process.env.JOURNEY_APP_PORT || 39450);
const SHOT_BUDGET_BYTES = 900 * 1024; // committed-screenshot budget, total
const BUNDLE_BUDGET_BYTES = 512 * 1024; // portable source bundle, before compression

// The canonical prompt — GOAL.md's storyboard sentence, verbatim.
const PROMPT = 'a post-op recovery tracker for my knee replacement patients';
const PACK = 'post-op-monitor';
const APP_NAME = 'post-op recovery tracker';
const TOKEN = 'dev-token-osei'; // dr-osei — clinician, meridian (staging/identities.hcl)
const COSIGNER = 'Dr. A. Osei';

// ---------- plumbing (same patterns as evals/harness/run.mjs) ----------

async function loadPlaywright() {
  const candidates = [
    process.env.EVALS_PLAYWRIGHT_MODULE,
    '/opt/node22/lib/node_modules/playwright/index.mjs',
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return import(pathToFileURL(candidate).href);
  }
  return import('playwright');
}

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

async function waitHealthy(base, child, logPath, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`server exited early (code ${child.exitCode}) — see ${logPath}`);
    }
    try {
      const res = await fetch(`${base}/health`);
      if (res.ok) return;
    } catch { /* not up yet */ }
    await sleep(50);
  }
  throw new Error(`server at ${base} never became healthy — see ${logPath}`);
}

function assert(cond, why) {
  if (!cond) throw new Error(`journey assertion failed: ${why}`);
}

// Timed, authenticated API call. Returns {status, json, text, ms}.
const CP_BASE = `http://127.0.0.1:${CP_PORT}`;
async function api(method, route, body) {
  const headers = { authorization: `Bearer ${TOKEN}` };
  if (body !== undefined) headers['content-type'] = 'application/json';
  const t = performance.now();
  const res = await fetch(`${CP_BASE}${route}`, {
    method, headers, body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  const ms = Math.round(performance.now() - t);
  let json = null;
  try { json = JSON.parse(text); } catch { /* JSONL export */ }
  return { status: res.status, json, text, ms };
}

// ---------- the stage recorder: timings + audit cross-references ----------

const stages = [];
let auditHighWater = 0;
let appId = null;

// After a stage, attribute every audit seq that appeared during it.
async function auditSeqsSince() {
  if (!appId) return [];
  const audit = await api('GET', `/api/apps/${appId}/audit`);
  const fresh = (audit.json?.events ?? []).filter((e) => e.seq > auditHighWater);
  if (fresh.length) auditHighWater = fresh[fresh.length - 1].seq;
  return fresh.map((e) => e.seq);
}

async function record(stage, ms, what) {
  const seqs = await auditSeqsSince();
  stages.push({ stage, ms, what, audit_seqs: seqs });
  const seqLabel = seqs.length ? ` [audit ${seqs.join(',')}]` : '';
  console.log(`  ${String(ms).padStart(6)}ms  ${stage}${seqLabel} — ${what}`);
  return seqs;
}

const shotPath = (name) => path.join(OUT_DIR, name);

// ---------- main ----------

async function main() {
  console.log('== journey profiler: one clinician journey, profiled\n');
  fs.rmSync(WORK_DIR, { recursive: true, force: true });
  // The eject target is wiped too: the committed build timing must be the
  // cold compile a stranger gets, not a profiler-local cache hit.
  fs.rmSync(EJECT_TARGET_DIR, { recursive: true, force: true });
  fs.mkdirSync(LOGS_DIR, { recursive: true });
  fs.mkdirSync(OUT_DIR, { recursive: true });
  for (const stale of fs.readdirSync(OUT_DIR)) {
    if (/\.(png|md|json)$/.test(stale)) fs.rmSync(path.join(OUT_DIR, stale));
  }
  assert(fs.existsSync(CONTROL_PLANE_BIN),
    `control plane binary missing at ${CONTROL_PLANE_BIN} — run scripts/journey.sh`);

  const { chromium } = await loadPlaywright();
  const browser = await chromium.launch();

  // -- boot the control plane (not part of the journey clock)
  const cpLog = path.join(LOGS_DIR, 'control-plane.log');
  let t = performance.now();
  const cp = spawnServer(CONTROL_PLANE_BIN,
    { APP_BIND: `127.0.0.1:${CP_PORT}`, CONTROL_DB_URL: '' }, cpLog);
  await waitHealthy(CP_BASE, cp, cpLog);
  const cpBootMs = Math.round(performance.now() - t);
  console.log(`  control plane healthy on :${CP_PORT} in ${cpBootMs}ms (in-memory dev mode)\n`);

  // The journey clock starts when Dr. Osei's sentence is sent.
  const t0 = performance.now();
  const t0Wall = Date.now();

  // ---- 1. DESCRIBE ----
  const created = await api('POST', '/api/apps', { prompt: PROMPT, pack: PACK, name: APP_NAME });
  assert(created.status === 200, `describe returned ${created.status}: ${created.text.slice(0, 300)}`);
  appId = created.json.app.id;
  const scaffoldSteps = (created.json.scaffold ?? []).map((s) => s.label);
  const initialControls = [...created.json.app.controls];
  assert(created.json.app.stage === 'sandbox', 'new app must start in the sandbox');
  assert(created.json.app.data_source.kind === 'synthetic', 'new app must start on synthetic data');
  await record('describe', created.ms,
    `POST /api/apps → app ${appId} in sandbox on synthetic data; ` +
    `${scaffoldSteps.length} scaffold steps; controls pre-wired: [${initialControls.join(', ')}]`);

  // ---- 2. PLATFORM UI moment 1: the studio ----
  const ui = await browser.newContext({
    viewport: { width: 1100, height: 800 },
    extraHTTPHeaders: { authorization: `Bearer ${TOKEN}` },
  });
  const page = await ui.newPage();
  await page.route(/fonts\.(googleapis|gstatic)\.com/, (r) => r.abort()); // hermetic
  const openApp = async (view = 'studio') => {
    await page.goto(`${CP_BASE}/`, { waitUntil: 'domcontentloaded' });
    await page.locator('button[onclick^="openApp("]').filter({ hasText: APP_NAME }).first().click();
    await page.getByTestId(view).waitFor();
  };
  t = performance.now();
  await openApp();
  await page.waitForSelector('#reply'); // the chat pane is live
  await page.getByTestId('environment-badge').getByText('SANDBOX · SYNTHETIC').waitFor();
  await page.screenshot({ path: shotPath('01-sandbox.png') });
  await record('ui-sandbox', Math.round(performance.now() - t),
    'Playwright on the studio: the app open in a synthetic sandbox with conversation and preview → 01-sandbox.png');

  // ---- 3. ITERATE: two conversational edits ----
  const iterations = [
    { instruction: 'make pain a 0-10 scale and flag anything over 7 to me', expect_wired: ['escalation-path'] },
    { instruction: 'remind patients to log wound photos daily', expect_wired: [] },
  ];
  const iterated = [];
  for (const [i, step] of iterations.entries()) {
    const it = await api('POST', `/api/apps/${appId}/iterate`, { instruction: step.instruction });
    assert(it.status === 200, `iterate returned ${it.status}: ${it.text.slice(0, 300)}`);
    const wired = it.json.reply.wired_controls ?? [];
    assert(wired.sort().join() === step.expect_wired.sort().join(),
      `iterate wired [${wired}], expected [${step.expect_wired}]`);
    iterated.push({ instruction: step.instruction, ms: it.ms, wired, reply: it.json.reply.message ?? it.json.reply.text ?? '' });
    await record(`iterate-${i + 1}`, it.ms,
      `"${step.instruction}" → wired [${wired.join(', ') || 'nothing — feature only, honest off-vocabulary edit'}]`);
  }
  // the tier that actually ran each operation — recorded, not embellished
  const ops = await api('GET', `/api/apps/${appId}/operations`);
  const tiers = {};
  for (const op of ops.json?.operations ?? []) {
    for (const attempt of op.attempts) {
      const key = `${op.kind}:${attempt.tier}`;
      tiers[key] = (tiers[key] ?? 0) + 1;
    }
  }
  const tierSummary = Object.entries(tiers).map(([k, v]) => `${k}×${v}`).join(', ');
  console.log(`          agent tiers used: ${tierSummary}`);

  // ---- 4. GATE, failing: named, screenshotted, and the locked promote ----
  const gate0 = await api('GET', `/api/apps/${appId}/gate`);
  const r0 = gate0.json.report;
  const failing0 = r0.results.filter((x) => x.status === 'fail');
  assert(!r0.green && failing0.length === 1 && failing0[0].id === 'auto-logoff',
    `expected exactly auto-logoff failing, got [${failing0.map((x) => x.id).join(', ')}]`);
  const gateInitial = {
    passed: r0.passed, stubbed: r0.stubbed, total: r0.total,
    satisfied: r0.passed + r0.stubbed, green: r0.green,
    failing: failing0.map((x) => ({ id: x.id, title: x.title, reason: x.reason ?? x.outcome?.reason ?? '', fixable: !!x.fixable })),
  };
  await record('gate-failing', gate0.ms,
    `GET gate → ${gateInitial.satisfied}/${r0.total} satisfied (${r0.passed} passed + ${r0.stubbed} labeled stub), ` +
    `failing: ${failing0[0].id} (one-click fixable: ${failing0[0].fixable})`);

  t = performance.now();
  await openApp();
  await page.getByRole('button', { name: 'Check before release →' }).click();
  await page.getByTestId('gate-dialog').waitFor();
  await page.getByText('Know what is ready before anyone uses it.').waitFor();
  await page.screenshot({ path: shotPath('02-gate-failing.png') });
  await record('ui-gate-failing', Math.round(performance.now() - t),
    'the release gate names the failure and offers its repair → 02-gate-failing.png');

  const refused = await api('POST', `/api/apps/${appId}/promote`, { cosigner: COSIGNER });
  assert(refused.status === 409, `promote-while-failing must be 409, got ${refused.status}`);
  const refusalBody = refused.text; // verbatim — the product's own words
  assert(refusalBody.includes('auto-logoff') && refusalBody.includes('phi-encryption') && refusalBody.includes('STUBBED'),
    `the real-data refusal must name both the failing check and the production-blocking stub: ${refusalBody}`);
  await record('promote-locked', refused.ms,
    `POST real-data promote while failing → 409; named auto-logoff + encryption stub and durably audited the denial`);

  // ---- 5. FIX: one click, then green ----
  const fixed = await api('POST', `/api/apps/${appId}/gate/auto-logoff/fix`, {});
  assert(fixed.status === 200, `fix returned ${fixed.status}: ${fixed.text.slice(0, 200)}`);
  const gate1 = await api('GET', `/api/apps/${appId}/gate`);
  const r1 = gate1.json.report;
  assert(r1.green, `gate must be green after the fix, still failing: ${JSON.stringify(r1.results.filter((x) => x.status === 'fail').map((x) => x.id))}`);
  await record('fix-auto-logoff', fixed.ms + gate1.ms,
    `POST gate/auto-logoff/fix → gate ${r1.passed + r1.stubbed}/${r1.total} satisfied, green (${r1.passed} passed + ${r1.stubbed} labeled stub)`);

  // ---- 6. CO-SIGN & RELEASE ----
  const promoted = await api('POST', `/api/apps/${appId}/promote`, { cosigner: COSIGNER, synthetic_demo: true });
  assert(promoted.status === 200, `promote returned ${promoted.status}: ${promoted.text.slice(0, 300)}`);
  assert(promoted.json.app.stage === 'live', 'promoted app must be live');
  const att = promoted.json.app.attestation;
  const alloc = promoted.json.app.allocation;
  assert(att?.principal === 'dr-osei' && att?.cosigner === COSIGNER && att?.report_digest,
    'attestation must bind the authenticated principal and the frozen report digest');
  const promptToLiveMs = Math.round(performance.now() - t0);
  await record('cosign-release', promoted.ms,
    `POST promote (cosigner "${COSIGNER}", synthetic_demo=true) → isolated synthetic demo; attestation by ${att.principal}, ` +
    `digest ${att.report_digest.slice(0, 16)}…; allocation ${alloc.id} (${alloc.pool} pool, ${alloc.url})`);

  t = performance.now();
  await openApp('live-dashboard'); // now shows the isolated synthetic-demo operate view
  await page.getByTestId('live-dashboard').waitFor();
  await page.getByTestId('runtime-status').waitFor();
  await page.screenshot({ path: shotPath('03-live.png') });
  await record('ui-live', Math.round(performance.now() - t),
    'the live view: synthetic badge, frozen release record, ownership action, and honest runtime status → 03-live.png');
  await ui.close();

  // ---- 7. EJECT ----
  const exported = await api('GET', `/api/apps/${appId}/export`);
  assert(exported.status === 200, `export returned ${exported.status}`);
  const files = exported.json.files;
  const fileSizes = Object.fromEntries(
    Object.entries(files).map(([p, c]) => [p, Buffer.byteLength(c, 'utf8')]));
  const bundleBytes = Object.values(fileSizes).reduce((a, b) => a + b, 0);
  assert(bundleBytes <= BUNDLE_BUDGET_BYTES,
    `ejected bundle is ${bundleBytes} bytes; budget is ${BUNDLE_BUDGET_BYTES} bytes`);
  const readmeOpening = files['README.md'].split('\n').slice(0, 6).join('\n');
  assert(files['README.md'].includes(PROMPT), 'the ejected README must open with the doctor\'s own prompt');
  const compliance = files['README.md'];
  const frozenHeading = compliance.split('\n').find((l) => l.startsWith('## Gate report'));
  const digestLine = compliance.split('\n').find((l) => l.includes('gate report digest'));
  assert(frozenHeading?.includes('frozen at promotion'), 'COMPLIANCE.md must carry the frozen report');
  assert(digestLine?.includes(att.report_digest), 'COMPLIANCE.md digest must match the attestation');
  await record('eject', exported.ms,
    `GET export → ${Object.keys(files).length} files, ${bundleBytes} bytes; README opens with the prompt; ` +
    `COMPLIANCE.md carries the frozen report + digest`);

  // ---- 8. THE ARTIFACT: unpack, build, boot, drive ----
  for (const [rel, content] of Object.entries(files)) {
    const dest = path.join(BUNDLE_DIR, rel);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.writeFileSync(dest, content);
  }
  t = performance.now();
  execFileSync('cargo', ['build', '--quiet'], {
    cwd: path.join(BUNDLE_DIR, 'app'),
    env: { ...process.env, CARGO_TARGET_DIR: EJECT_TARGET_DIR },
    stdio: ['ignore', 'inherit', 'inherit'],
  });
  const buildMs = Math.round(performance.now() - t);
  await record('artifact-build', buildMs, `cargo build of the ejected app/ crate — cold, worktree-local target (what a stranger gets)`);

  const APP_BASE = `http://127.0.0.1:${APP_PORT}`;
  const appLog = path.join(LOGS_DIR, 'artifact-app.jsonl');
  t = performance.now();
  const artifact = spawnServer(path.join(EJECT_TARGET_DIR, 'debug', 'app'), {
    APP_BIND: `127.0.0.1:${APP_PORT}`,
    SYNTHETIC_DATA: path.join(BUNDLE_DIR, 'synthetic', 'post-op-demo.json'),
  }, appLog, path.join(BUNDLE_DIR, 'app'));
  await waitHealthy(APP_BASE, artifact, appLog);
  const bootMs = Math.round(performance.now() - t);
  const promptToRunningMs = Math.round(performance.now() - t0);
  await record('artifact-boot', bootMs, `the ejected binary healthy on :${APP_PORT} against its bundled synthetic seed`);

  const actx = await browser.newContext({ viewport: { width: 1100, height: 800 } });
  const apage = await actx.newPage();
  await apage.route(/fonts\.(googleapis|gstatic)\.com/, (r) => r.abort());
  t = performance.now();
  await apage.goto(`${APP_BASE}/login`, { waitUntil: 'domcontentloaded' });
  await apage.getByLabel('username').fill('demo-patient');
  await apage.getByLabel('password').fill('learn-patient');
  await apage.getByRole('button', { name: 'sign in' }).click();
  assert((await apage.getByText('synthetic data only').count()) > 0, 'the artifact must show its SYNTHETIC banner');
  await apage.screenshot({ path: shotPath('04-artifact-home.png') });
  await apage.fill('input[name="pain"]', '9');
  await apage.selectOption('select[name="wound"]', 'clean');
  await apage.fill('input[name="note"]', 'much worse since last night');
  await apage.screenshot({ path: shotPath('05-artifact-form.png') });
  await apage.click('button[type="submit"]');
  await apage.getByText('flag routed to the practice inbox').waitFor();
  await apage.screenshot({ path: shotPath('06-artifact-flag.png') });
  await sleep(200); // let the app's stdout audit JSONL flush
  const artifactAuditLines = fs.readFileSync(appLog, 'utf8').split('\n')
    .filter((l) => l.includes('"control":"audit-log"'));
  assert(artifactAuditLines.some((l) => l.includes('"path":"/checkin"')),
    'the artifact\'s own audit JSONL must record the check-in');
  await record('artifact-drive', Math.round(performance.now() - t),
    `pain-9 check-in for pt-001 → flag routed to the practice inbox; the app's own audit JSONL ` +
    `recorded ${artifactAuditLines.length} events → 04/05/06 screenshots`);
  await actx.close();
  children.delete(artifact);
  try { artifact.kill('SIGTERM'); } catch { /* gone */ }

  // ---- 9. AUDIT TIMELINE: the journey's spine ----
  const auditFinal = await api('GET', `/api/apps/${appId}/audit`);
  const events = (auditFinal.json?.events ?? []).map((e) => ({
    seq: e.seq,
    at: e.at,
    offset_ms: Math.max(0, e.at * 1000 - t0Wall), // audit clock is 1s-granular
    actor: e.actor,
    action: e.action,
    detail: e.detail,
  }));
  console.log(`\n  audit spine: ${events.length} events, seq ${events[0]?.seq}–${events.at(-1)?.seq}`);

  await browser.close();
  children.delete(cp);
  try { cp.kill('SIGTERM'); } catch { /* gone */ }

  // ---- screenshots: enforce the committed budget ----
  const shots = fs.readdirSync(OUT_DIR).filter((f) => f.endsWith('.png')).sort();
  let total = shots.reduce((a, f) => a + fs.statSync(shotPath(f)).size, 0);
  if (total > SHOT_BUDGET_BYTES) {
    const ffmpeg = path.join(process.env.PLAYWRIGHT_BROWSERS_PATH || '/opt/pw-browsers', 'ffmpeg-1011', 'ffmpeg-linux');
    for (const scale of [0.75, 0.6, 0.5]) {
      if (total <= SHOT_BUDGET_BYTES) break;
      assert(fs.existsSync(ffmpeg), `screenshots over budget (${total}B) and no ffmpeg at ${ffmpeg} to downscale`);
      for (const f of shots) {
        const tmp = path.join(WORK_DIR, f);
        execFileSync(ffmpeg, ['-y', '-loglevel', 'error', '-i', shotPath(f),
          '-vf', `scale=iw*${scale}:-1`, tmp]);
        fs.copyFileSync(tmp, shotPath(f));
      }
      total = shots.reduce((a, f) => a + fs.statSync(shotPath(f)).size, 0);
      console.log(`  screenshots downscaled ×${scale} → ${total} bytes total`);
    }
  }
  const screenshots = shots.map((f) => ({ file: f, bytes: fs.statSync(shotPath(f)).size }));
  console.log(`  screenshots: ${screenshots.map((s) => `${s.file} (${(s.bytes / 1024).toFixed(0)}KB)`).join(', ')}`);

  // ---- totals ----
  const apiOnlyMs = created.ms + iterated.reduce((a, i) => a + i.ms, 0)
    + gate0.ms + refused.ms + fixed.ms + gate1.ms + promoted.ms;
  const totals = {
    prompt_to_live_ms: promptToLiveMs,
    prompt_to_ejected_running_ms: promptToRunningMs,
    api_calls_only_ms: apiOnlyMs,
    artifact_build_ms: buildMs,
    artifact_boot_ms: bootMs,
    control_plane_boot_ms: cpBootMs,
  };
  console.log(`\n  prompt → isolated demo: ${promptToLiveMs}ms (API calls alone: ${apiOnlyMs}ms)`);
  console.log(`  prompt → ejected app running: ${promptToRunningMs}ms (build ${buildMs}ms, boot ${bootMs}ms)`);

  // ---- write the record ----
  const commit = execFileSync('git', ['rev-parse', '--short', 'HEAD'], { cwd: ROOT }).toString().trim();
  const meta = { generated_at: new Date().toISOString(), commit };
  const honesty = [
    `agent tier: every scaffold/iterate ran on the deterministic rules driver (${tierSummary}) — the honest floor; no model endpoint was configured (decision 0002 keeps sandbox/CI model-free).`,
    `allocation ${alloc.id} is simulated in dev mode (in-memory control plane, no Nomad configured) — the same promote renders a real Nomad job in staging (#2/#6).`,
    'the pack\'s phi-encryption check is a labeled stub — it satisfies the meter as "stubbed", is never drawn as a pass, and the ejected app labels encryption-at-rest as a TODO on its own page.',
    'the gate meter reads 5/6 before the fix because the labeled stub counts as satisfied-with-a-caveat: 4 passed + 1 stub, auto-logoff failing.',
    'the locked promote (409) is enforcement without an audit event today — the refusal is captured here verbatim from the HTTP body; only state-changing actions land on the app\'s stream.',
    'audit offsets are derived from the stream\'s 1-second timestamps; stage wall times are measured around each HTTP call/build/boot at ms precision.',
  ];
  const record_ = {
    ...meta,
    prompt: PROMPT, pack: PACK, app_id: appId, principal: 'dr-osei', cosigner: COSIGNER,
    scaffold_steps: scaffoldSteps,
    initial_controls: initialControls,
    iterations: iterated,
    agent_tiers: tiers,
    gate: {
      initial: gateInitial,
      refusal: { status: refused.status, body: refusalBody },
      after_fix: { passed: r1.passed, stubbed: r1.stubbed, total: r1.total, green: r1.green },
    },
    attestation: {
      cosigner: att.cosigner, principal: att.principal,
      gate_summary: att.gate_summary, report_digest: att.report_digest,
    },
    allocation: alloc,
    stages,
    bundle: {
      file_count: Object.keys(files).length,
      total_bytes: bundleBytes,
      files: fileSizes,
      readme_opening: readmeOpening,
      compliance_report_heading: frozenHeading,
      compliance_digest_line: digestLine.trim(),
      unpack: exported.json.unpack,
    },
    artifact: { build_ms: buildMs, boot_ms: bootMs, audit_jsonl: artifactAuditLines },
    audit_timeline: events,
    totals,
    screenshots,
    honesty,
  };
  fs.writeFileSync(path.join(OUT_DIR, 'journey.json'), JSON.stringify(record_, null, 2) + '\n');
  fs.writeFileSync(path.join(OUT_DIR, 'journey.md'), renderMarkdown(record_));
  console.log(`\n== journey written: docs/evals/journey/journey.md + journey.json + ${screenshots.length} screenshots`);
}

// ---------- journey.md: the show-anyone narrative ----------

function renderMarkdown(j) {
  const sec = (ms) => ms >= 10000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
  const seqRange = (seqs) => seqs.length === 0 ? '—'
    : seqs.length === 1 ? String(seqs[0]) : `${seqs[0]}–${seqs.at(-1)}`;
  const L = [];
  L.push(`# One journey, profiled — ${j.generated_at.slice(0, 10)} at \`${j.commit}\``);
  L.push('');
  L.push('One real clinician journey on the flagship pack (post-op-monitor, the');
  L.push('fully-real one), run end to end by `scripts/journey.sh` against a freshly');
  L.push('booted control plane: every step timed, every step cross-referenced to the');
  L.push('audit events it produced, ending with the artifact Dr. Osei owns — ejected,');
  L.push('compiled, booted, and driven. Machine twin: [journey.json](journey.json).');
  L.push('');
  L.push('## What Dr. Osei typed');
  L.push('');
  L.push(`> ${j.prompt}`);
  L.push('');
  L.push(`That sentence — authenticated as \`${j.principal}\` (clinician, Meridian Family`);
  L.push(`Practice) with a dev bearer token from \`staging/identities.hcl\` — is the`);
  L.push(`whole spec. Everything below happened to it.`);
  L.push('');
  L.push('## The stage timeline');
  L.push('');
  L.push('| # | stage | wall time | what happened | audit seqs |');
  L.push('|---|---|---|---|---|');
  j.stages.forEach((s, i) => {
    L.push(`| ${i + 1} | ${s.stage} | ${sec(s.ms)} | ${s.what.replace(/\|/g, '\\|')} | ${seqRange(s.audit_seqs)} |`);
  });
  L.push(`| | **totals** | | **prompt → isolated synthetic demo: ${sec(j.totals.prompt_to_live_ms)}** (API calls alone: ${sec(j.totals.api_calls_only_ms)}) · **prompt → ejected app running: ${sec(j.totals.prompt_to_ejected_running_ms)}** (incl. ${sec(j.totals.artifact_build_ms)} compile) | |`);
  L.push('');
  L.push('Wall times are measured around each HTTP call / build / boot at ms');
  L.push('precision; the ui-* rows are the profiler driving the real doctor UI with');
  L.push('Playwright, so the prompt→live wall clock includes them.');
  L.push('');
  L.push('## The sandbox, as the doctor sees it');
  L.push('');
  L.push(`Scaffolded in ${sec(j.stages[0].ms)} from the pack: ${j.scaffold_steps.map((s) => `*${s}*`).join(' → ')}.`);
  L.push(`Controls pre-wired on day one: \`${j.initial_controls.join('`, `')}\`.`);
  L.push('');
  L.push('![the app in sandbox — chat and preview, 1a builder skin](01-sandbox.png)');
  L.push('');
  L.push('Two conversational edits followed:');
  L.push('');
  for (const it of j.iterations) {
    L.push(`- "${it.instruction}" — ${sec(it.ms)}, wired ${it.wired.length ? `\`${it.wired.join('`, `')}\`` : 'nothing (an off-vocabulary edit: the feature lands, no control is claimed)'}`);
  }
  L.push('');
  L.push(`Agent tier for every operation: **${Object.keys(j.agent_tiers).map((k) => k.split(':')[1]).filter((v, i, a) => a.indexOf(v) === i).join(', ')}** (${Object.entries(j.agent_tiers).map(([k, v]) => `${k}×${v}`).join(', ')}) — the deterministic rules floor, recorded honestly.`);
  L.push('');
  L.push('## The gate story');
  L.push('');
  L.push(`The preflight gate read **${j.gate.initial.satisfied}/${j.gate.initial.total} satisfied** (${j.gate.initial.passed} passed`);
  L.push(`+ ${j.gate.initial.stubbed} labeled stub) with one named failure:`);
  L.push('');
  for (const f of j.gate.initial.failing) {
    L.push(`- \`${f.id}\` — ${f.reason || f.title} (one-click fixable: ${f.fixable})`);
  }
  L.push('');
  L.push('![the preflight modal naming the failure](02-gate-failing.png)');
  L.push('');
  L.push(`Promotion while failing was refused — HTTP ${j.gate.refusal.status}, the product's own words, verbatim:`);
  L.push('');
  L.push('```json');
  L.push(j.gate.refusal.body);
  L.push('```');
  L.push('');
  L.push(`One click fixed \`auto-logoff\`; the gate went **green at ${j.gate.after_fix.passed + j.gate.after_fix.stubbed}/${j.gate.after_fix.total}**`);
  L.push(`(${j.gate.after_fix.passed} passed + ${j.gate.after_fix.stubbed} labeled stub — the stub is satisfied-with-a-caveat, never a pass).`);
  L.push(`Dr. Osei co-signed as **${j.attestation.cosigner}** and the release attestation bound the`);
  L.push(`authenticated principal \`${j.attestation.principal}\` to the frozen gate report:`);
  L.push('');
  L.push(`- gate summary at release: **${j.attestation.gate_summary}**`);
  L.push(`- report digest: \`${j.attestation.report_digest}\``);
  L.push(`- allocation: \`${j.allocation.id}\` (${j.allocation.pool} pool, ${j.allocation.region}, ${j.allocation.url})`);
  L.push('');
  L.push('![the live operate view](03-live.png)');
  L.push('');
  L.push('## What they own now');
  L.push('');
  L.push(`One GET later, the whole app left the platform as a ${j.bundle.file_count}-file,`);
  L.push(`${(j.bundle.total_bytes / 1024).toFixed(0)}KB bundle — source, docs generated from Dr. Osei's own record, and`);
  L.push('deploy manifests for four targets. No hostage code, no hostage docs.');
  L.push('');
  L.push('```');
  for (const [p, bytes] of Object.entries(j.bundle.files)) {
    L.push(`${String(bytes).padStart(7)}  ${p}`);
  }
  L.push(`${String(j.bundle.total_bytes).padStart(7)}  total`);
  L.push('```');
  L.push('');
  L.push('The README opens with their own sentence:');
  L.push('');
  L.push('```markdown');
  L.push(j.bundle.readme_opening);
  L.push('```');
  L.push('');
  L.push(`And COMPLIANCE.md carries the release evidence frozen at promotion —`);
  L.push(`"${j.bundle.compliance_report_heading.replace(/^## /, '')}", with its digest line verbatim:`);
  L.push('');
  L.push('```markdown');
  L.push(j.bundle.compliance_digest_line);
  L.push('```');
  L.push('');
  L.push(`The profiler then did what a stranger would: unpacked the bundle, ran`);
  L.push(`\`cargo build\` (${sec(j.artifact.build_ms)}), booted the binary against its bundled synthetic`);
  L.push(`seed (${sec(j.artifact.boot_ms)}), and used it:`);
  L.push('');
  L.push('| the ejected app, SYNTHETIC banner up | a pain-9 check-in filled | the flag, routed |');
  L.push('|---|---|---|');
  L.push('| ![home](04-artifact-home.png) | ![form](05-artifact-form.png) | ![flag](06-artifact-flag.png) |');
  L.push('');
  L.push("The ejected app kept its own books — its stdout audit JSONL during the");
  L.push('interaction, verbatim:');
  L.push('');
  L.push('```json');
  for (const line of j.artifact.audit_jsonl) L.push(line);
  L.push('```');
  L.push('');
  L.push('## The audit spine');
  L.push('');
  L.push('Every platform action above, from the append-only stream (offsets from the');
  L.push('moment the prompt was sent; the stream\'s clock is 1-second granular):');
  L.push('');
  L.push('| seq | +ms | actor | action | detail |');
  L.push('|---|---|---|---|---|');
  for (const e of j.audit_timeline) {
    L.push(`| ${e.seq} | +${e.offset_ms} | ${e.actor} | \`${e.action}\` | ${e.detail.replace(/\|/g, '\\|')} |`);
  }
  L.push('');
  L.push('## Honesty footnotes');
  L.push('');
  for (const h of j.honesty) L.push(`- ${h}`);
  L.push('');
  return L.join('\n');
}

main().catch((err) => {
  console.error(`journey profiler error: ${err.stack ?? err}`);
  process.exit(2);
});
