// M4's exit test on the real desktop app (12-roadmap): a plugin published to the registry
// by a third party installs on a fresh machine, with its permissions shown and enforced.
//
// The third party is simulated on loopback: test-plugins/registry-fixture makes the
// publisher "acme" a key, signs the probe test plugin as `acme.probe` and serves the
// registry. The app (and the core it starts) use that registry as their default. Fresh data
// directories make it a fresh machine.
//
// Needs, in the environment (scripts/desktop/e2e.sh sets them up on Linux; CI on Windows):
// KK_APP, KK_CORE_BIN, KK_FAKE_AGENT_DIR, KK_BUNDLE, KK_REGISTRY_FIXTURE (the
// registry-fixture binary), and tauri-driver on PATH.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { basename, delimiter, join } from 'node:path';
import { createInterface } from 'node:readline';
import { after, before, test } from 'node:test';

import { Session } from './webdriver.mjs';

const need = (name) => {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set; see the comment at the top of this file`);
  return v;
};
const windows = process.platform === 'win32';
const work = mkdtempSync(join(windows ? tmpdir() : '/tmp', 'kk4-'));
const socket = windows ? `\\\\.\\pipe\\kumikit-e2e4-${process.pid}` : join(work, 's.sock');
const picks = join(work, 'picks');
const project = join(work, 'proj');
const bundle = need('KK_BUNDLE');

let driver;
let registry;
let indexUrl;
const answers = [];

/** Sends one command to the registry fixture and waits for its `ok`. */
function fixture(command) {
  return new Promise((ok, fail) => {
    answers.push((line) => (line.startsWith('ok') ? ok(line.slice(2).trim()) : fail(new Error(`${command}: ${line}`))));
    registry.stdin.write(`${command}\n`);
  });
}

before(async () => {
  mkdirSync(project);
  writeFileSync(picks, '');
  registry = spawn(need('KK_REGISTRY_FIXTURE'), [join(work, 'reg')], { stdio: ['pipe', 'pipe', 'inherit'] });
  const lines = createInterface({ input: registry.stdout });
  indexUrl = await new Promise((ok, fail) => {
    lines.once('line', (l) => (l.startsWith('ready ') ? ok(l.slice(6)) : fail(new Error(l))));
    registry.once('exit', (c) => fail(new Error(`the registry fixture exited (${c})`)));
  });
  lines.on('line', (l) => answers.shift()?.(l));
  const src = join(work, 'acme-probe');
  await fixture(`probe ${src} 0.1.0`);
  await fixture(`publish ${src}`);
  await fixture('starter acme.probe');
  await fixture('build');

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
    KUMIKIT_DEFAULT_REGISTRY: indexUrl,
    KUMIKIT_TEST_PICKS: picks,
    WEBKIT_DISABLE_DMABUF_RENDERER: '1',
  };
  driver = spawn('tauri-driver', [], { env, stdio: 'inherit' });
  for (let i = 0; i < 200; i++) {
    const up = await new Promise((r) => net.connect(4444, '127.0.0.1').once('connect', function () { this.end(); r(true); }).once('error', () => r(false)));
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
      c.write('{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{"protocol":"0.9","client":{"name":"e2e","version":"0"}}}\n');
      c.write('{"jsonrpc":"2.0","id":2,"method":"core.shutdown","params":{}}\n');
    });
    c.once('close', () => done());
    c.resume();
  });
  driver?.kill();
  registry?.stdin.end();
  rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
});

const session = () => Session.start('http://127.0.0.1:4444', { 'tauri:options': { application: need('KK_APP') } });
const pick = (path) => writeFileSync(picks, path + '\n');
const step = (what) => process.stderr.write(`[e2e m4] ${what}\n`);

async function palette(b, title) {
  await b.keys(['Control', 'k']);
  await b.type(await b.wait('input[aria-label="Command"]'), title);
  await b.keys('Enter');
}

/** Clicks the button labelled `label` inside the list item that mentions `item`. */
async function clickIn(b, item, label) {
  await b.until(
    () =>
      b.execute(
        `const li = [...document.querySelectorAll('li')].find((l) => l.textContent.includes(arguments[0]) && [...l.querySelectorAll('button')].some((x) => x.textContent.includes(arguments[1])));
         if (!li) return false;
         [...li.querySelectorAll('button')].find((x) => x.textContent.includes(arguments[1])).click();
         return true;`,
        [item, label],
      ),
    `${label} in ${item}`,
  );
}

let current;

test('a third-party plugin from the registry, on the desktop app', { timeout: 300_000 }, async () => {
  step('starting the app');
  const b = await session();
  current = b;
  try {
    await flow(b);
  } catch (e) {
    const out = process.env.KK_ARTIFACTS ?? work;
    mkdirSync(out, { recursive: true });
    await current.screenshot(join(out, 'failure-m4.png')).catch(() => {});
    step(`page text: ${await current.execute('return document.body.textContent').catch((x) => String(x))}`);
    throw e;
  }
});

async function flow(b) {
  await b.wait('footer', 'Connected');
  step('the welcome asks before contacting the registry');
  const welcome = await b.wait('section[aria-labelledby="kk-welcome-title"]', 'Browse the starter pack');
  assert.match(await b.text(welcome), new RegExp(new URL(indexUrl).host.replace(/\./g, '\\.')));
  assert.equal(await fixture('hits'), '0', 'a fresh app contacts no registry by itself');
  await b.click(await b.wait('section[aria-labelledby="kk-welcome-title"] button', 'Browse the starter pack'));

  step('installing acme.probe from the starter pack');
  await b.wait('section[aria-labelledby="kk-find-plugins"] li', 'acme.probe');
  assert.notEqual(await fixture('hits'), '0');
  await clickIn(b, 'acme.probe', 'Install on');
  const dialog = await b.wait('[role="dialog"]', 'Grant and enable');
  const shown = await b.text(dialog);
  assert.match(shown, /the publisher’s key in the registry/);
  assert.match(shown, /https:\/\/example\.com\/acme\/probe/);
  assert.match(shown, /can start the `fake-acp-agent` program, which runs with your full user access/);
  await b.click(await b.wait('[role="dialog"] button', 'Grant and enable'));
  await b.until(async () => (await b.text(await b.wait('section[aria-labelledby="kk-core-plugins"] li', 'acme.probe'))).includes('Enabled'), 'acme.probe to be enabled');

  step('a thread with it');
  pick(project);
  await palette(b, 'Open a project folder');
  await b.wait('nav[aria-label="Open workspaces"] button', 'proj');
  await b.wait('select[aria-label="Agent"] option', 'Acme Probe');
  const send = async (text) => {
    await b.type(await b.wait('#kk-chat-input'), text);
    await b.keys('Enter');
  };

  step('what was granted works');
  await send('report fake-acp-agent');
  // The reply is the agent's working directory as the core canonicalized it (on Windows
  // the long form of a short temp path), so look for the test directory's own name.
  await b.wait('main', basename(work));

  step('anything else is refused with permission_denied');
  await send('spawn sh');
  await b.wait('main', 'permission_denied: `sh` is not in permissions.spawn');
  await send('secret anthropic-api-key');
  await b.wait('main', 'permission_denied: secret `anthropic-api-key` is not in permissions.secrets');
  await b.end();
}
