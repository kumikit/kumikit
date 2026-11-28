// M7 exit test on the real desktop app: installs a recorded ACP provider, runs a turn,
// schedules a `/later` deferred message in the queue panel, asks a `/btw` side question
// in the inspector with Insert into main, and uses Edit on a sent prompt to open a fork
// with the prompt preloaded in the composer.
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
  if (!v) throw new Error(`${name} is not set`);
  return v;
};
const windows = process.platform === 'win32';
const work = mkdtempSync(join(windows ? tmpdir() : '/tmp', 'kk7-'));
const socket = windows ? `\\\\.\\pipe\\kumikit-e2e7-${process.pid}` : join(work, 's.sock');
const picks = join(work, 'picks');
const project = join(work, 'proj');
const bundle = need('KK_BUNDLE');

let driver;

before(async () => {
  mkdirSync(project);
  writeFileSync(join(project, 'notes.md'), '# Notes\n');
  mkdirSync(join(work, 'provider'));
  writeFileSync(picks, '');
  const transcript = join(ROOT, 'plugins/provider-acp/tests/transcripts/claude-text.jsonl').replaceAll('\\', '\\\\');
  writeFileSync(
    join(work, 'provider', 'plugin.toml'),
    `id = "test.recorded7"
name = "Recorded M7 Agent"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "agent", name = "Recorded M7 Agent", acp = { command = "fake-acp-agent", args = ["replay", "${transcript}"] } }]
`,
  );
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
  for (let i = 0; i < 200; i++) {
    const up = await new Promise((r) =>
      net
        .connect(4444, '127.0.0.1')
        .once('connect', function () {
          this.end();
          r(true);
        })
        .once('error', () => r(false)),
    );
    if (up) return;
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error('tauri-driver did not start');
});

after(async () => {
  await new Promise((done) => {
    const c = net.connect(socket);
    c.once('error', () => done());
    c.once('connect', () => {
      c.write('{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{"protocol":"0.13","client":{"name":"e2e","version":"0"}}}\n');
      c.write('{"jsonrpc":"2.0","id":2,"method":"core.shutdown","params":{}}\n');
    });
    c.once('close', () => done());
    c.resume();
  });
  driver?.kill();
  rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
});

const session = () => Session.start('http://127.0.0.1:4444', { 'tauri:options': { application: need('KK_APP') } });
const pick = (path) => writeFileSync(picks, path + '\n');
const step = (what) => process.stderr.write(`[e2e m7] ${what}\n`);

async function palette(b, title) {
  await b.keys(['Control', 'k']);
  await b.type(await b.wait('input[aria-label="Command"]'), title);
  await b.keys('Enter');
}

/** Settings → Agents, where agents are installed (06-client, Agents). */
const AGENTS = 'section[aria-labelledby="kk-agents-installed"]';
let current;

test('M7 threads flow on the desktop app: /later, /btw side thread, and edit fork', { timeout: 300_000 }, async () => {
  step('starting the app');
  const b = await session();
  current = b;
  try {
    await flow(b);
  } catch (e) {
    const out = process.env.KK_ARTIFACTS ?? work;
    mkdirSync(out, { recursive: true });
    await current.screenshot(join(out, 'failure-m7.png')).catch(() => {});
    step(`page text: ${await current.execute('return document.body.textContent').catch((x) => String(x))}`);
    throw e;
  }
});

async function flow(b) {
  await b.wait('footer', 'Connected');
  step('installing recorded ACP provider');
  await palette(b, 'Agents: install, remove and configure');
  pick(join(work, 'provider'));
  await b.click(await b.wait(`${AGENTS} button`, 'Install from folder'));
  await b.click(await b.wait('[role="dialog"] button', 'Grant and enable'));
  await b.until(
    async () => (await b.text(await b.wait(`${AGENTS} li`, 'test.recorded7'))).includes('Enabled'),
    'test.recorded7 enabled',
  );

  step('opening workspace and running turn 1');
  pick(project);
  await palette(b, 'Open a project folder');
  await b.wait('nav[aria-label="Open workspaces"] button', 'proj');
  await b.chooseAgent('Recorded M7 Agent');

  const input = await b.wait('main textarea[aria-label="Message"], main #kk-chat-input');
  await b.type(input, 'First M7 prompt');
  await b.keys('Enter');
  await b.wait('main', 'hello from kumikit');

  step('scheduling /later deferred message');
  await b.type(await b.wait('main textarea[aria-label="Message"], main #kk-chat-input'), '/later 15m check the build');
  await b.keys('Enter');
  await b.wait('main section[aria-label="Queued messages"]', 'check the build');
  await b.wait('main section[aria-label="Queued messages"]', 'In 15m');

  step('asking /btw side question');
  await b.type(await b.wait('main textarea[aria-label="Message"], main #kk-chat-input'), '/btw side check');
  await b.keys('Enter');
  await b.click(await b.wait('button', 'Insert into main'));
  await b.until(
    () =>
      b.execute(
        'const ta = document.querySelector("main textarea"); return Boolean(ta && ta.value.includes("hello from kumikit"));',
      ),
    'composer received side answer',
  );

  step('editing prompt in fork');
  await b.click(await b.wait('main button[aria-label="Edit prompt in fork"]'));
  await b.wait('main [role="note"]', 'Workspace files were not rewound');
  const preloaded = await b.execute('const ta = document.querySelector("main textarea"); return ta ? ta.value : "";');
  assert.equal(preloaded, 'First M7 prompt');
  await b.end();
}
