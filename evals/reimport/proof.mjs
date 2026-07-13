#!/usr/bin/env node

import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const evidence = path.join(root, '.evals', 'reimport');
const bundleDir = path.join(evidence, 'owned-bundle');
const reportPath = path.join(evidence, 'report.json');
const logPath = path.join(evidence, 'control-plane.log');
const port = Number(process.env.REIMPORT_PROOF_PORT || 39520);
const baseUrl = `http://127.0.0.1:${port}`;

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

async function api(method, route, body) {
  const response = await fetch(`${baseUrl}${route}`, {
    method,
    headers: body ? { 'content-type': 'application/json' } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const value = await response.json();
  if (!response.ok) throw new Error(`${method} ${route} returned ${response.status}: ${JSON.stringify(value)}`);
  return value;
}

async function waitHealthy(child) {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`control plane exited ${child.exitCode}`);
    try {
      const response = await fetch(`${baseUrl}/health`);
      if (response.ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error('control plane did not become healthy');
}

function writeBundle(files) {
  for (const [relative, content] of Object.entries(files)) {
    assert.match(relative, /^[A-Za-z0-9._+/-]+$/);
    assert.equal(relative.startsWith('/'), false);
    assert.equal(relative.split('/').some((part) => !part || part === '.' || part === '..'), false);
    const target = path.resolve(bundleDir, relative);
    assert.equal(target.startsWith(`${bundleDir}${path.sep}`), true);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, content, { flag: 'wx' });
  }
}

async function main() {
  fs.rmSync(evidence, { recursive: true, force: true });
  fs.mkdirSync(bundleDir, { recursive: true });
  const log = fs.openSync(logPath, 'w');
  const child = spawn(path.join(root, 'target', 'debug', 'rust-proof-service'), [], {
    cwd: root,
    env: { ...process.env, APP_BIND: `127.0.0.1:${port}` },
    stdio: ['ignore', log, log],
  });
  try {
    await waitHealthy(child);
    const created = await api('POST', '/api/apps', {
      prompt: 'synthetic clean-room reimport proof',
      pack: 'post-op-monitor',
      name: 'Owned reimport proof',
    });
    const exported = await api('GET', `/api/apps/${created.app.id}/export`);
    writeBundle(exported.files);
    fs.appendFileSync(path.join(bundleDir, 'server', 'src', 'main.rs'), '\n// reimport-server-sentinel\n');
    fs.appendFileSync(path.join(bundleDir, 'web', 'src', 'routes', '+page.svelte'), '\n<!-- reimport-svelte-sentinel -->\n');

    execFileSync('node', ['scripts/reimport.mjs'], {
      cwd: bundleDir,
      env: { ...process.env, PRACTICE_STUDIO_URL: baseUrl },
      stdio: 'inherit',
    });
    const importedResult = JSON.parse(fs.readFileSync(path.join(bundleDir, 'reimport-result.json'), 'utf8'));
    const importedId = importedResult.app.id;
    const workspace = await api('GET', `/api/apps/${importedId}/workspace`);
    const reexported = await api('GET', `/api/apps/${importedId}/export`);
    const checks = [
      { id: 'fresh-id', passed: importedId !== created.app.id },
      { id: 'private-synthetic-sandbox', passed: importedResult.app.stage === 'sandbox' && importedResult.app.data_source.kind === 'synthetic' },
      { id: 'no-inherited-authority', passed: importedResult.app.attestation === null && importedResult.app.allocation === null },
      { id: 'exact-verification-digest', passed: importedResult.verification.passed && importedResult.source_digest === importedResult.verification.workspace_digest },
      { id: 'rust-source-preserved', passed: workspace.accepted.files['server/src/main.rs'].includes('reimport-server-sentinel') && reexported.files['server/src/main.rs'] === workspace.accepted.files['server/src/main.rs'] },
      { id: 'svelte-source-preserved', passed: workspace.accepted.files['web/src/routes/+page.svelte'].includes('reimport-svelte-sentinel') && reexported.files['web/src/routes/+page.svelte'] === workspace.accepted.files['web/src/routes/+page.svelte'] },
      { id: 'gemma-not-invoked', passed: workspace.plan_agent == null && workspace.generation_agent == null },
    ];

    const { chromium } = await loadPlaywright();
    const browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({
      viewport: { width: 1280, height: 900 },
      extraHTTPHeaders: { authorization: 'Bearer dev-token-osei' },
    });
    await page.goto(`${baseUrl}/`, { waitUntil: 'networkidle' });
    await page.getByText('Owned reimport proof (template)', { exact: false }).first().waitFor();
    await page.screenshot({ path: path.join(evidence, 'imported-starter.png'), fullPage: true });
    await browser.close();
    checks.push({ id: 'import-visible-in-studio', passed: true });

    fs.writeFileSync(reportPath, `${JSON.stringify({ source_app: created.app.id, imported_app: importedId, checks }, null, 2)}\n`);
    if (!checks.every((check) => check.passed)) throw new Error(`reimport checks failed; see ${reportPath}`);
  } finally {
    child.kill('SIGTERM');
    fs.closeSync(log);
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
