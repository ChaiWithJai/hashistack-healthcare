#!/usr/bin/env node

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const OUT = path.join(ROOT, '.evals', 'treatment-preview');
const BIN = path.join(process.env.CARGO_TARGET_DIR || path.join(ROOT, 'target'), 'debug', 'rust-proof-service');
const PORT = Number(process.env.TREATMENT_PREVIEW_PORT || 42000 + (process.pid % 1000));
const BASE = `http://127.0.0.1:${PORT}`;

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

function assert(condition, detail) {
  if (!condition) throw new Error(`treatment preview proof failed: ${detail}`);
}

async function waitHealthy(child) {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`control plane exited ${child.exitCode}`);
    try {
      if ((await fetch(`${BASE}/health`)).ok) return;
    } catch {}
    await sleep(50);
  }
  throw new Error(`control plane did not become healthy at ${BASE}`);
}

async function workspaceState(page, appId) {
  return page.evaluate(async (id) => {
    const workspace = await (await fetch(`/api/apps/${id}/workspace`)).json();
    const audit = await (await fetch(`/api/apps/${id}/audit`)).json();
    return {
      digest: workspace.accepted.digest,
      version: workspace.accepted.version,
      auditEvents: audit.events.length,
      acceptedRecipe: JSON.parse(workspace.accepted.files['web/src/lib/treatment.json']).treatment.id,
      acceptedFeatures: JSON.parse(workspace.accepted.files['web/src/lib/treatment.json']).features,
    };
  }, appId);
}

async function appIdFor(page, name) {
  return page.evaluate(async (appName) => {
    const body = await (await fetch('/api/apps')).json();
    return body.apps.find((app) => app.name === appName)?.id ?? null;
  }, name);
}

async function returnHome(page) {
  const home = page.getByRole('button', { name: '← Your ideas' });
  if (await home.count()) await home.click();
}

async function createAcceptedPreview(page, { name, treatment, contextFirst = false, addFeatureBeforeAccept = false }) {
  await returnHome(page);
  await page.getByRole('textbox', { name: /What would make your day/ }).fill(name);
  await page.getByRole('button', { name: 'Show me 3 approaches →' }).click();
  await page.getByRole('radio', { name: new RegExp(treatment) }).click();
  if (contextFirst) {
    await page.getByRole('radio', { name: /Follow-up first/ }).click();
    await page.getByRole('textbox', { name: /Anything you want to emphasize/ })
      .fill('Show why the practice inbox was notified.');
  }
  const appId = await appIdFor(page, name);
  assert(appId, `could not find ${name}`);
  if (addFeatureBeforeAccept) {
    const iterated = await page.evaluate(async (id) => {
      const response = await fetch(`/api/apps/${id}/iterate`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ instruction: 'add a synthetic handoff confirmation step' }),
      });
      return { ok: response.ok, body: await response.json() };
    }, appId);
    assert(iterated.ok, `pre-accept feature mutation failed: ${JSON.stringify(iterated.body)}`);
  }
  await page.getByTestId('build-treatment').click();
  await page.getByTestId('candidate-review').waitFor();
  await page.getByTestId('accept-candidate').click();
  const preview = page.getByTestId('treatment-workspace-preview');
  await preview.waitFor();
  return { preview, appId };
}

async function provePreviewIsLocal(page, appId, action) {
  const before = await workspaceState(page, appId);
  const requests = [];
  const record = (request) => requests.push(request.url());
  page.on('request', record);
  await action();
  await page.waitForTimeout(25);
  page.off('request', record);
  const after = await workspaceState(page, appId);
  assert(before.digest === after.digest, 'preview action changed the accepted checkpoint');
  assert(before.auditEvents === after.auditEvents, 'preview action wrote an audit event');
  assert(requests.length === 0, `preview action made network requests: ${requests.join(', ')}`);
  return { before, after };
}

async function main() {
  assert(fs.existsSync(BIN), `binary missing at ${BIN}; run cargo build first`);
  fs.mkdirSync(OUT, { recursive: true });
  const log = fs.openSync(path.join(OUT, 'control-plane.log'), 'w');
  const server = spawn(BIN, [], {
    cwd: ROOT,
    env: { ...process.env, APP_BIND: `127.0.0.1:${PORT}`, CONTROL_DB_URL: '' },
    stdio: ['ignore', log, log],
  });
  let browser;
  try {
    await waitHealthy(server);
    const { chromium } = await loadPlaywright();
    browser = await chromium.launch({ headless: true });
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const page = await context.newPage();
    await page.route(/fonts\.(googleapis|gstatic)\.com/, (route) => route.abort());
    await page.goto(BASE, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: /post-op monitor web/ }).click();

    const results = [];

    const event = await createAcceptedPreview(page, {
      name: 'event preview proof', treatment: 'Event timeline', contextFirst: true,
    });
    assert(await event.preview.getAttribute('data-treatment-id') === 'event-timeline', 'event recipe mismatch');
    const eventContext = await event.preview.getByTestId('treatment-preview-context').boundingBox();
    const eventTools = await event.preview.getByLabel('Filter synthetic events').boundingBox();
    assert(eventContext.y < eventTools.y, 'context-first did not put explanation before the task');
    const eventState = await provePreviewIsLocal(page, event.appId, async () => {
      await event.preview.getByRole('button', { name: 'Toggle review' }).first().click();
      await event.preview.getByRole('button', { name: 'reviewed', exact: true }).click();
      assert(await event.preview.getByTestId('preview-timeline-item').count() === 1, 'review filter failed');
    });
    await page.screenshot({ path: path.join(OUT, 'event-timeline.png'), fullPage: true });

    const frozenFeature = await event.preview.getByTestId('preview-timeline-item').first().locator('b').textContent();
    const mutation = 'add a synthetic discharge call checklist';
    const iterated = await page.evaluate(async ({ appId, mutation }) => {
      const response = await fetch(`/api/apps/${appId}/iterate`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ instruction: mutation }),
      });
      return { ok: response.ok, body: await response.json() };
    }, { appId: event.appId, mutation });
    assert(iterated.ok, `feature mutation failed: ${JSON.stringify(iterated.body)}`);
    await returnHome(page);
    await page.getByRole('button', { name: /event preview proof/ }).click();
    const frozenPreview = page.getByTestId('treatment-workspace-preview');
    await frozenPreview.waitFor();
    assert(await frozenPreview.getByText(frozenFeature, { exact: true }).count() === 1, 'accepted feature snapshot changed after app iteration');
    assert(await frozenPreview.getByText(/synthetic discharge call checklist/i).count() === 0, 'mutable app feature leaked into accepted preview');

    await page.getByRole('textbox', { name: 'Ask for one change…' }).fill('focus on one next action');
    await page.getByRole('button', { name: 'Send' }).click();
    await page.getByRole('radio', { name: /Approach Focused task/ }).click();
    await page.getByTestId('build-treatment').click();
    await page.getByTestId('candidate-review').waitFor();
    await page.getByTestId('revise-candidate').click();
    await page.getByTestId('treatment-screen').waitFor();
    await returnHome(page);
    await page.getByRole('button', { name: /event preview proof/ }).click();
    const reopened = page.getByTestId('treatment-workspace-preview');
    await reopened.waitFor();
    assert(await reopened.getAttribute('data-treatment-id') === 'event-timeline', 'rejected selection replaced accepted preview');
    const afterReject = await workspaceState(page, event.appId);
    assert(afterReject.acceptedRecipe === 'event-timeline', 'rejected selection replaced accepted bytes');
    results.push({ recipe: 'event-timeline', contextFirst: true, ...eventState.after, rejectedSelectionIsolated: true, featureMutationIsolated: true });

    const guided = await createAcceptedPreview(page, { name: 'guided preview proof', treatment: 'Guided worklist' });
    assert(await guided.preview.getAttribute('data-treatment-id') === 'guided-worklist', 'guided recipe mismatch');
    const guidedItem = await guided.preview.getByTestId('preview-worklist-item').first().boundingBox();
    const guidedContext = await guided.preview.getByTestId('treatment-preview-context').boundingBox();
    assert(guidedItem.y < guidedContext.y, 'task-first did not put guided task before explanation');
    const guidedState = await provePreviewIsLocal(page, guided.appId, async () => {
      await guided.preview.getByRole('button', { name: 'Mark reviewed' }).first().click();
      assert(await guided.preview.getByText('Reviewed', { exact: true }).count() === 1, 'worklist toggle failed');
    });
    results.push({ recipe: 'guided-worklist', taskFirst: true, ...guidedState.after });

    const focused = await createAcceptedPreview(page, {
      name: 'focused preview proof', treatment: 'Focused task', addFeatureBeforeAccept: true,
    });
    assert(await focused.preview.getAttribute('data-treatment-id') === 'focused-task', 'focused recipe mismatch');
    const focusPanel = await focused.preview.getByTestId('preview-focused-task').boundingBox();
    const focusContext = await focused.preview.getByTestId('treatment-preview-context').boundingBox();
    assert(focusPanel.y < focusContext.y, 'task-first did not put focused task before explanation');
    const first = await focused.preview.getByRole('heading', { level: 3 }).textContent();
    const focusedState = await provePreviewIsLocal(page, focused.appId, async () => {
      await focused.preview.getByRole('button', { name: 'Continue' }).click();
      const second = await focused.preview.getByRole('heading', { level: 3 }).textContent();
      assert(first !== second, 'focused task did not advance');
    });
    assert(focusedState.after.acceptedFeatures.length > 1, 'focused checkpoint needs multiple frozen steps');
    const restored = await page.evaluate(async (id) => {
      const response = await fetch(`/api/apps/${id}/restore`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ version: 1 }),
      });
      return { ok: response.ok, body: await response.json() };
    }, focused.appId);
    assert(restored.ok, `feature shrink restore failed: ${JSON.stringify(restored.body)}`);
    assert(restored.body.features.length < focusedState.after.acceptedFeatures.length, 'restore did not shrink mutable app features');
    await returnHome(page);
    await page.getByRole('button', { name: /focused preview proof/ }).click();
    const restoredFocus = page.getByTestId('preview-focused-task');
    await restoredFocus.waitFor();
    const back = restoredFocus.getByRole('button', { name: 'Back' });
    while (await back.isEnabled()) await back.click();
    for (let index = 0; index < focusedState.after.acceptedFeatures.length - 1; index += 1) {
      await restoredFocus.getByRole('button', { name: 'Continue' }).click();
    }
    assert(
      await restoredFocus.getByRole('heading', { level: 3 }).textContent()
        === focusedState.after.acceptedFeatures.at(-1),
      'mutable feature shrink made a frozen focused step unreachable',
    );
    results.push({ recipe: 'focused-task', taskFirst: true, ...focusedState.after, featureShrinkIsolated: true });

    const report = { passed: true, modelBoundary: 'Gemma-only; preview is verified deterministic UI', results };
    fs.writeFileSync(path.join(OUT, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
  } finally {
    if (browser) await browser.close();
    server.kill('SIGTERM');
    fs.closeSync(log);
  }
}

main().catch((error) => {
  console.error(error.stack || error);
  process.exitCode = 1;
});
