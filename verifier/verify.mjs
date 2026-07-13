import { spawn } from 'node:child_process';
import { mkdir, readFile, readdir, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { chromium } from 'playwright';

const CHECKS = [
  'workspace.structure.v1',
  'web.svelte-check.v1',
  'web.svelte-build.v1',
  'server.cargo-test.v1',
  'browser.synthetic-smoke.v1'
];

const limits = {
  'web.svelte-check.v1': 30_000,
  'web.svelte-build.v1': 45_000,
  'server.cargo-test.v1': 60_000,
  'browser.synthetic-smoke.v1': 20_000
};

function args() {
  const values = new Map();
  for (let index = 2; index < process.argv.length; index += 2) {
    values.set(process.argv[index], process.argv[index + 1]);
  }
  const workspace = values.get('--workspace');
  const report = values.get('--report');
  if (!workspace || !report || !report.startsWith(`${workspace}/`)) {
    throw new Error('workspace and in-workspace report paths are required');
  }
  return { workspace, report };
}

function clean(value) {
  return value.replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 600);
}

async function run(command, argv, cwd, timeoutMs, env = {}) {
  return await new Promise((resolve) => {
    const child = spawn(command, argv, {
      cwd,
      env: { PATH: process.env.PATH, HOME: '/tmp/home', CI: '1', ...env },
      detached: true,
      stdio: ['ignore', 'pipe', 'pipe']
    });
    let output = '';
    const capture = (chunk) => { if (output.length < 65_536) output += chunk.toString('utf8'); };
    child.stdout.on('data', capture);
    child.stderr.on('data', capture);
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      try { process.kill(-child.pid, 'SIGKILL'); } catch {}
    }, timeoutMs);
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      resolve({ passed: code === 0 && !timedOut, detail: clean(timedOut ? 'timed out' : output || `exit ${code ?? signal}`) });
    });
  });
}

async function structure(workspace) {
  const required = [
    'web/package.json',
    'web/src/routes/+page.svelte',
    'server/Cargo.toml',
    'server/src/main.rs',
    'synthetic'
  ];
  for (const path of required) {
    try { await stat(join(workspace, path)); }
    catch { return { passed: false, detail: `missing ${path}` }; }
  }
  const manifest = JSON.parse(await readFile(join(workspace, 'web/package.json'), 'utf8'));
  for (const script of ['check', 'build']) {
    if (typeof manifest.scripts?.[script] !== 'string') return { passed: false, detail: `missing web script ${script}` };
  }
  const page = await readFile(join(workspace, 'web/src/routes/+page.svelte'), 'utf8');
  if (!page.includes('$state(')) return { passed: false, detail: 'Svelte 5 rune $state is missing' };
  return { passed: true, detail: 'required Svelte 5 and Rust workspace files are present' };
}

async function browserSmoke(workspace) {
  const preview = spawn('vite', ['preview', '--host', '127.0.0.1', '--port', '4173'], {
    cwd: join(workspace, 'web'),
    env: { PATH: process.env.PATH, HOME: '/tmp/home', CI: '1' },
    detached: true,
    stdio: 'ignore'
  });
  try {
    const browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
    page.on('request', (request) => {
      const url = new URL(request.url());
      if (!['127.0.0.1', 'localhost'].includes(url.hostname)) errors.push(`external request ${url.hostname}`);
    });
    let response;
    for (let attempt = 0; attempt < 50; attempt += 1) {
      try { response = await page.goto('http://127.0.0.1:4173', { waitUntil: 'networkidle', timeout: 1000 }); break; }
      catch { await new Promise((resolve) => setTimeout(resolve, 100)); }
    }
    const body = await page.locator('body').innerText();
    const hasHeading = await page.locator('h1').count() > 0;
    const synthetic = /synthetic/i.test(body);
    await page.keyboard.press('Tab');
    const focused = await page.evaluate(() => document.activeElement !== document.body);
    await browser.close();
    const passed = Boolean(response?.ok() && hasHeading && synthetic && focused && errors.length === 0);
    return { passed, detail: passed ? 'page loaded with heading, synthetic warning, keyboard focus, and no browser errors' : clean(errors.join('; ') || 'browser contract failed') };
  } finally {
    try { process.kill(-preview.pid, 'SIGKILL'); } catch {}
  }
}

async function main() {
  const { workspace, report } = args();
  await mkdir('/tmp/home', { recursive: true });
  const workspaceModules = join(workspace, 'web/node_modules');
  await rm(workspaceModules, { recursive: true, force: true });
  await mkdir(workspaceModules);
  for (const entry of await readdir('/opt/practice-studio/node_modules')) {
    await symlink(join('/opt/practice-studio/node_modules', entry), join(workspaceModules, entry));
  }
  const checks = [];
  const first = await structure(workspace);
  checks.push({ id: CHECKS[0], ...first });
  const commands = [
    ['web.svelte-check.v1', 'npm', ['run', 'check'], join(workspace, 'web')],
    ['web.svelte-build.v1', 'npm', ['run', 'build'], join(workspace, 'web')]
  ];
  for (const [id, command, argv, cwd] of commands) {
    if (!checks.at(-1).passed) checks.push({ id, passed: false, detail: `not run: prerequisite ${checks.at(-1).id} failed` });
    else checks.push({ id, ...(await run(command, argv, cwd, limits[id])) });
  }
  if (!checks.at(-1).passed) {
    checks.push({ id: 'server.cargo-test.v1', passed: false, detail: `not run: prerequisite ${checks.at(-1).id} failed` });
  } else {
    const locked = await run('cargo', ['generate-lockfile', '--offline', '--manifest-path', 'server/Cargo.toml'], workspace, 15_000);
    checks.push({
      id: 'server.cargo-test.v1',
      ...(locked.passed
        ? await run('cargo', ['test', '--offline', '--locked', '--manifest-path', 'server/Cargo.toml'], workspace, limits['server.cargo-test.v1'])
        : locked)
    });
  }
  if (!checks.at(-1).passed) checks.push({ id: CHECKS[4], passed: false, detail: `not run: prerequisite ${checks.at(-1).id} failed` });
  else checks.push({ id: CHECKS[4], ...(await browserSmoke(workspace)) });
  await writeFile(report, JSON.stringify({ checks }) + '\n', { mode: 0o600 });
  process.exit(checks.every((check) => check.passed) ? 0 : 1);
}

main().catch(async (error) => {
  try {
    const { report } = args();
    await writeFile(report, JSON.stringify({ checks: CHECKS.map((id, index) => ({ id, passed: false, detail: index === 0 ? clean(error.message) : `not run: prerequisite ${CHECKS[index - 1]} failed` })) }) + '\n');
  } catch {}
  process.exit(1);
});
