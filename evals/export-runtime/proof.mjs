#!/usr/bin/env node

import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const BUNDLE = path.join(ROOT, '.evals', 'bundles', 'post-op-01-precise-physician');
const EVIDENCE = path.join(ROOT, '.evals', 'export-runtime');
const PORT = Number(process.env.EXPORT_RUNTIME_PORT || 39490);
const IMAGE = `practice-studio-export-proof:${process.pid}`;
const CONTAINER = `practice-studio-export-proof-${process.pid}`;
const LOG = path.join(EVIDENCE, 'container.log');
const REPORT = path.join(EVIDENCE, 'report.json');

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

function docker(args, options = {}) {
  return execFileSync('docker', args, { encoding: 'utf8', ...options });
}

async function waitHealthy(base, child) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`container log follower exited ${child.exitCode}`);
    try {
      const response = await fetch(`${base}/health`, { headers: { accept: 'application/json' } });
      const health = response.headers.get('content-type')?.includes('application/json')
        ? await response.json()
        : null;
      if (response.ok && health?.status === 'ok') return;
    } catch {}
    await sleep(200);
  }
  throw new Error(`exported container did not become healthy; see ${LOG}`);
}

async function main() {
  if (!fs.existsSync(path.join(BUNDLE, 'Dockerfile'))) {
    throw new Error(`representative export is missing: ${BUNDLE}`);
  }
  fs.rmSync(EVIDENCE, { recursive: true, force: true });
  fs.mkdirSync(EVIDENCE, { recursive: true });

  docker(['build', '--tag', IMAGE, BUNDLE], { stdio: 'inherit' });
  docker([
    'run', '--detach', '--name', CONTAINER,
    '--publish', `127.0.0.1:${PORT}:8080`, IMAGE,
  ]);
  const logFile = fs.openSync(LOG, 'w');
  const logs = spawn('docker', ['logs', '--follow', CONTAINER], {
    stdio: ['ignore', logFile, logFile],
  });
  const base = `http://127.0.0.1:${PORT}`;
  const checks = [];
  try {
    await waitHealthy(base, logs);
    const runtimeUser = docker(['inspect', '--format', '{{.Config.User}}', CONTAINER]).trim();
    checks.push({
      id: 'non-root-runtime',
      passed: runtimeUser !== '' && runtimeUser !== '0' && runtimeUser !== 'root',
      detail: runtimeUser,
    });
    const { chromium } = await loadPlaywright();
    const browser = await chromium.launch({ headless: true });
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const page = await context.newPage();
    const external = [];
    page.on('request', (request) => {
      const url = new URL(request.url());
      if (!['127.0.0.1', 'localhost'].includes(url.hostname)) external.push(url.hostname);
    });

    const root = await page.goto(`${base}/login`, { waitUntil: 'domcontentloaded' });
    checks.push({ id: 'clinical-workflow', passed: root?.ok() && await page.getByText('Demo learning credentials').count() === 1 });
    await page.screenshot({ path: path.join(EVIDENCE, 'clinical-workflow.png'), fullPage: true });

    const workspace = await page.goto(`${base}/workspace/`, { waitUntil: 'networkidle' });
    const connected = await page.getByText('Rust service connected', { exact: true }).count() === 1;
    checks.push({ id: 'workspace-same-origin-rust', passed: Boolean(workspace?.ok() && connected) });
    await page.screenshot({ path: path.join(EVIDENCE, 'workspace-connected.png'), fullPage: true });

    await page.getByRole('button', { name: 'Pain 8', exact: true }).click();
    await page.getByRole('button', { name: 'Send today’s check-in', exact: true }).click();
    const signIn = page.getByRole('link', { name: 'Sign in as the synthetic patient', exact: true });
    await signIn.waitFor();
    checks.push({ id: 'submission-requires-patient-session', passed: await signIn.count() === 1 });
    await signIn.click();
    await page.getByLabel('username').fill('demo-patient');
    await page.getByLabel('password').fill('learn-patient');
    await page.getByRole('button', { name: 'sign in', exact: false }).click();
    await page.waitForURL(`${base}/workspace/`);

    await page.getByRole('button', { name: 'Pain 8', exact: true }).click();
    await page.getByRole('button', { name: 'Send today’s check-in', exact: true }).click();
    const queued = page.getByText('Queued in the synthetic practice inbox.', { exact: true });
    await queued.waitFor();
    const painEight = page.getByText(/Pain 8\/10 was evaluated by Rust/);
    checks.push({
      id: 'svelte-rust-escalation',
      passed: await queued.count() === 1 && await painEight.count() === 1,
      detail: 'pain 8 was submitted from Svelte and Rust returned a queued synthetic flag',
    });
    await page.screenshot({ path: path.join(EVIDENCE, 'svelte-rust-escalation.png'), fullPage: true });

    const workspaceReload = await page.reload({ waitUntil: 'networkidle' });
    const reconnected = await page.getByText('Rust service connected', { exact: true }).count() === 1;
    checks.push({ id: 'workspace-reload', passed: Boolean(workspaceReload?.ok() && reconnected) });

    await page.goto(`${base}/login`, { waitUntil: 'domcontentloaded' });
    await page.getByLabel('username').fill('demo-clinician');
    await page.getByLabel('password').fill('learn-clinician');
    await page.getByRole('button', { name: 'sign in', exact: false }).click();
    await page.waitForURL(`${base}/clinician`);
    const inboxFlag = page.getByText(/pain 8\/10 at or over threshold 7/);
    await inboxFlag.waitFor();
    checks.push({
      id: 'clinician-sees-same-flag',
      passed: await inboxFlag.count() === 1,
      detail: 'the role-protected Rust inbox contains the Svelte-submitted flag',
    });
    await page.screenshot({ path: path.join(EVIDENCE, 'clinician-inbox-flag.png'), fullPage: true });

    const missingAsset = await context.request.get(`${base}/workspace/_app/not-a-real-asset.js`);
    checks.push({ id: 'missing-static-asset', passed: missingAsset.status() === 404 });

    const rust404 = await context.request.get(`${base}/not-a-real-route`);
    checks.push({ id: 'rust-route-boundary', passed: rust404.status() === 404 });
    await sleep(100);
    const audit = fs.readFileSync(LOG, 'utf8');
    checks.push({
      id: 'checkin-domain-audit',
      passed: (audit.match(/post_op\.checkin\.accepted/g) || []).length === 1
        && (audit.match(/post_op\.escalation\.queued/g) || []).length === 1,
      detail: 'one linked check-in event and one escalation event',
    });
    checks.push({ id: 'no-external-requests', passed: external.length === 0, detail: [...new Set(external)].join(', ') });

    await context.close();
    await browser.close();
    fs.writeFileSync(REPORT, `${JSON.stringify({ image: IMAGE, checks }, null, 2)}\n`);
    if (!checks.every((check) => check.passed)) throw new Error(`combined export checks failed; see ${REPORT}`);
  } finally {
    logs.kill('SIGTERM');
    fs.closeSync(logFile);
    try { docker(['rm', '--force', CONTAINER]); } catch {}
    try { docker(['image', 'rm', '--force', IMAGE]); } catch {}
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
