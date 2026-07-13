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
const port = Number(process.env.REIMPORT_PROOF_PORT || 24220);
const ownedAppPort = Number(process.env.REIMPORT_OWNED_APP_PORT || 24230);
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

function replaceExact(relative, before, after) {
  const target = path.join(bundleDir, relative);
  const content = fs.readFileSync(target, 'utf8');
  assert.equal(content.includes(before), true, `${relative} no longer contains the expected customization anchor`);
  fs.writeFileSync(target, content.replace(before, after));
}

async function waitForUrl(url, childLabel) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {}
    await sleep(100);
  }
  throw new Error(`${childLabel} did not become healthy at ${url}`);
}

async function proveChangedOwnedApp() {
  const image = `practice-owned-reimport-proof:${process.pid}`;
  const container = `practice-owned-reimport-proof-${process.pid}`;
  try {
    execFileSync('docker', ['build', '--tag', image, '.'], { cwd: bundleDir, stdio: 'inherit' });
    execFileSync('docker', [
      'run', '--detach', '--name', container,
      '--publish', `127.0.0.1:${ownedAppPort}:8080`, image,
    ], { cwd: bundleDir, stdio: 'inherit' });
    const ownedUrl = `http://127.0.0.1:${ownedAppPort}`;
    await waitForUrl(`${ownedUrl}/health`, 'changed owned app');
    execFileSync('npm', ['ci', '--ignore-scripts', '--no-audit', '--no-fund'], {
      cwd: path.join(bundleDir, 'web'),
      stdio: 'inherit',
    });
    execFileSync('node', ['tests/owned-app.mjs'], {
      cwd: path.join(bundleDir, 'web'),
      env: { ...process.env, OWNED_APP_URL: ownedUrl },
      stdio: 'inherit',
    });
    const report = JSON.parse(fs.readFileSync(path.join(bundleDir, 'web', 'test-results', 'report.json'), 'utf8'));
    assert.equal(report.passed, true, 'changed owned app browser journey failed');
    return report;
  } finally {
    try { execFileSync('docker', ['rm', '--force', container], { stdio: 'ignore' }); } catch {}
    try { execFileSync('docker', ['image', 'rm', '--force', image], { stdio: 'ignore' }); } catch {}
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

    // Follow the exported README's change map: change one observable behavior
    // across Rust, Svelte, the synthetic fixture, and its executable browser
    // contract. Pain 6 was below the original threshold; in this owned version
    // it must enter the synthetic practice inbox.
    replaceExact(
      'server/src/main.rs',
      'const PAIN_ESCALATION_THRESHOLD: u8 = 7;',
      'const PAIN_ESCALATION_THRESHOLD: u8 = 6;',
    );
    replaceExact(
      'web/src/lib/PostOpCheckIn.svelte',
      'Try pain 8 to see the Rust escalation rule route one synthetic flag.',
      'Try pain 6 to verify this owned practice threshold.',
    );
    replaceExact(
      'web/src/lib/treatment.json',
      '"label": "Guided worklist"',
      '"label": "Owned recovery worklist"',
    );
    replaceExact(
      'synthetic/post-op-demo.json',
      '{ "day": 1, "pain": 6, "wound": "clean", "note": "sore but managing with prescribed meds" }',
      '{ "day": 1, "pain": 5, "wound": "clean", "note": "owned threshold rehearsal: below 6" }',
    );
    replaceExact(
      'web/tests/owned-app.mjs',
      "await page.getByText('Rust service connected', { exact: true }).waitFor();",
      "await page.getByText('Rust service connected', { exact: true }).waitFor();\n  await page.getByText('Owned recovery worklist', { exact: true }).first().waitFor();",
    );
    const journeyPath = path.join(bundleDir, 'web', 'tests', 'owned-app.mjs');
    const journey = fs.readFileSync(journeyPath, 'utf8')
      .replaceAll('Pain 8', 'Pain 6')
      .replace('pain 8\\/10 at or over threshold 7', 'pain 6\\/10 at or over threshold 6');
    fs.writeFileSync(journeyPath, journey);
    fs.appendFileSync(
      path.join(bundleDir, 'README.md'),
      '\n\n## Owned customization proof\n\nThis version routes synthetic pain 6 to the practice inbox. The Rust threshold, Svelte prompt, synthetic boundary example, and browser journey changed together.\n',
    );
    const changedAppReport = await proveChangedOwnedApp();

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
      { id: 'changed-app-journey-passed', passed: changedAppReport.passed === true },
      { id: 'rust-behavior-preserved', passed: workspace.accepted.files['server/src/main.rs'].includes('PAIN_ESCALATION_THRESHOLD: u8 = 6') && reexported.files['server/src/main.rs'] === workspace.accepted.files['server/src/main.rs'] },
      { id: 'svelte-treatment-preserved', passed: workspace.accepted.files['web/src/lib/treatment.json'].includes('Owned recovery worklist') && reexported.files['web/src/lib/treatment.json'] === workspace.accepted.files['web/src/lib/treatment.json'] },
      { id: 'synthetic-fixture-preserved', passed: workspace.accepted.files['synthetic/post-op-demo.json'].includes('owned threshold rehearsal: below 6') && reexported.files['synthetic/post-op-demo.json'] === workspace.accepted.files['synthetic/post-op-demo.json'] },
      { id: 'browser-contract-preserved', passed: workspace.accepted.files['web/tests/owned-app.mjs'].includes("name: 'Pain 6'") && workspace.accepted.files['web/tests/owned-app.mjs'].includes('threshold 6') && reexported.files['web/tests/owned-app.mjs'] === workspace.accepted.files['web/tests/owned-app.mjs'] },
      { id: 'readme-record-preserved', passed: workspace.accepted.files['README.md'].includes('Owned customization proof') && reexported.files['README.md'] === workspace.accepted.files['README.md'] },
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

    fs.writeFileSync(reportPath, `${JSON.stringify({ source_app: created.app.id, imported_app: importedId, changed_app: changedAppReport, checks }, null, 2)}\n`);
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
