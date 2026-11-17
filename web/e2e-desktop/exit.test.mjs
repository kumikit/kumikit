// M2's exit test on the real desktop app (12-roadmap): from the app a user installs an
// ACP provider, starts a thread, approves a tool call and switches themes; closing and
// reopening the window resumes the running turn. The agent replays a recorded Claude
// Code transcript (fake-acp-agent), so no real agent or network is involved.
//
// Needs, in the environment (scripts/desktop/e2e.sh sets them up on Linux; CI on
// Windows): KK_APP (the kumikit-desktop binary built with --features test-hooks),
// KK_CORE_BIN, KK_FAKE_AGENT_DIR, KK_BUNDLE (target/bundle), and tauri-driver on PATH.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { delimiter, join, resolve } from 'node:path';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { Session } from './webdriver.mjs';

const ROOT = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const need = (name) => {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set; see the comment at the top of this file`);
  return v;
};
const windows = process.platform === 'win32';
const work = mkdtempSync(join(windows ? tmpdir() : '/tmp', 'kkd-'));
const socket = windows ? `\\\\.\\pipe\\kumikit-e2e-${process.pid}` : join(work, 's.sock');
const picks = join(work, 'picks');
const project = join(work, 'proj');
const bundle = need('KK_BUNDLE');

let driver;

before(async () => {
  mkdirSync(project);
  mkdirSync(join(work, 'provider'));
  writeFileSync(picks, '');
  const transcript = join(ROOT, 'plugins/provider-acp/tests/transcripts/claude-tool-allow.jsonl').replaceAll('\\', '\\\\');
  writeFileSync(
    join(work, 'provider', 'plugin.toml'),
    `id = "test.recorded"
name = "Recorded Claude Code"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "claude", name = "Recorded Claude Code", acp = { command = "fake-acp-agent", args = ["replay", "${transcript}"], mode = "default" } }]
`,
  );
  // The app, and the core it starts, see this environment.
  const env = {
    ...process.env,
    PATH: [need('KK_FAKE_AGENT_DIR'), process.env.PATH].join(delimiter),
    KUMIKIT_SOCKET: socket,
    KUMIKIT_CORE_DATA_DIR: join(work, 'core'),
    KUMIKIT_CLIENT_DATA_DIR: join(work, 'client'),
    KUMIKIT_CORE_BIN: need('KK_CORE_BIN'),
    KUMIKIT_UI_DIR: join(bundle, 'ui'),
    KUMIKIT_BUNDLED_UI_PLUGINS: join(bundle, 'ui-plugins'),
    KUMIKIT_BUNDLED_CORE_PLUGINS: join(bundle, 'core-plugins'),
    KUMIKIT_TEST_PICKS: picks,
    WEBKIT_DISABLE_DMABUF_RENDERER: '1',
  };
  driver = spawn('tauri-driver', [], { env, stdio: 'inherit' });
  // tauri-driver listens on 4444; wait for it rather than for a fixed time.
  for (let i = 0; i < 200; i++) {
    const up = await new Promise((r) => net.connect(4444, '127.0.0.1').once('connect', function () { this.end(); r(true); }).once('error', () => r(false)));
    if (up) return;
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error('tauri-driver did not start');
});

after(async () => {
  // Stop the core the app started (it outlives the window by design).
  await new Promise((done) => {
    const c = net.connect(socket);
    c.once('error', () => done());
    c.once('connect', () => {
      c.write('{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{"protocol":"0.4","client":{"name":"e2e","version":"0"}}}\n');
      c.write('{"jsonrpc":"2.0","id":2,"method":"core.shutdown","params":{}}\n');
    });
    c.once('close', () => done());
    c.resume();
  });
  driver?.kill();
  // On Windows the core may still hold its files for a moment after shutting down.
  rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
});

const session = () => Session.start('http://127.0.0.1:4444', { 'tauri:options': { application: need('KK_APP') } });

const pick = (path) => writeFileSync(picks, path + '\n');
const step = (what) => process.stderr.write(`[e2e] ${what}\n`);

const connected = (b) => b.wait('footer', 'Connected');

async function palette(b, title) {
  await b.keys(['Control', 'k']);
  await b.type(await b.wait('input[aria-label="Command"]'), title);
  await b.keys('Enter');
}

/** A button by its text inside `scope` (a CSS selector). */
const button = (b, scope, label) => b.wait(`${scope} button`, label);
const CORE = 'section[aria-labelledby="kk-core-plugins"]';
const APPROVAL = 'section[aria-label="Approval"]';

test('the exit criteria on the desktop app', { timeout: 300_000 }, async () => {
  step('starting the app');
  let b = await session();
  step('session started');
  try {
    await exitFlow(b);
  } catch (e) {
    await report(current ?? b);
    throw e;
  }
});

let current;

/** Saves what the window shows for a failure report. */
async function report(b) {
  const out = process.env.KK_ARTIFACTS ?? work;
  mkdirSync(out, { recursive: true });
  await b.screenshot(join(out, 'failure.png')).catch(() => {});
  step(`page text: ${await b.execute('return document.body.textContent').catch((x) => String(x))}`);
}

async function exitFlow(b) {
  current = b;
  await connected(b);

  step('installing the provider');
  // Install the declarative provider; provider-acp was seeded from the bundle.
  await palette(b, 'Manage plugins');
  await b.wait(`${CORE} li`, 'kumikit.provider-acp');
  pick(join(work, 'provider'));
  await b.click(await button(b, CORE, 'Install from folder…'));
  const dialog = await b.wait('[role="dialog"]', 'Grant and enable');
  assert.match(await b.text(dialog), /can start the `fake-acp-agent` program, which runs with your full user access/);
  await b.click(await button(b, '[role="dialog"]', 'Grant and enable'));
  await b.until(async () => (await b.text(await b.wait(`${CORE} li`, 'test.recorded'))).includes('Enabled'), 'the provider to be enabled');

  step('starting a thread');
  // A thread in a project, and a message that leads to an approval.
  pick(project);
  await palette(b, 'Open a project folder');
  await b.wait('nav[aria-label="Open workspaces"] button', 'proj');
  await palette(b, 'New thread');
  await b.click(await button(b, '[role="dialog"]', 'Start'));
  const box = await b.wait('#kk-chat-input');
  await b.type(box, 'Add a line to notes.txt');
  await b.keys('Enter');
  await b.wait(APPROVAL, 'Needs your approval');

  step('closing the window');
  // Close the window: the app exits, the core and the turn keep running.
  await b.end();
  const alive = await new Promise((r) => net.connect(socket).once('connect', function () { this.end(); r(true); }).once('error', () => r(false)));
  assert.equal(alive, true, 'the core keeps running after the window closes');

  step('reopening');
  // Reopen: the same turn, still waiting for the approval.
  b = await session();
  current = b;
  await connected(b);
  // Find the running thread from the sidebar. (Open tabs are client settings, kept in
  // the webview's storage; msedgedriver gives each session a fresh one.)
  await b.click(await b.wait('nav[aria-label="Workspaces"] button', 'proj'));
  await b.click(await b.wait('nav[aria-label="Workspaces"] li button', 'waiting for approval'));
  await b.wait(APPROVAL, 'Needs your approval');
  await b.click(await button(b, APPROVAL, 'Allow'));
  await b.wait(APPROVAL, 'Allowed');
  await b.wait('figcaption', 'Changed');

  step('switching themes');
  // Switch themes.
  const panel = () => b.execute("return getComputedStyle(document.documentElement).getPropertyValue('--kk-surface-panel').trim()");
  await palette(b, 'Theme: Kumikit Dark');
  await b.until(async () => (await panel()) === '#15171c', 'the dark theme');
  await palette(b, 'Theme: Kumikit Light');
  await b.until(async () => (await panel()) === '#ffffff', 'the light theme');
  await b.end();
}
