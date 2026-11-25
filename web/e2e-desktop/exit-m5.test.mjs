// M5 headline exit test on the real desktop app (E6, docs/plans/2026-11-19-m5-plugins.md):
// installs a tool plugin (`acme.tool`), an ACP agent granted `tools` (`acme.agent`) and a
// sandboxed panel (`acme.panel` requiring `acme.tool`) from the fixture registry, runs a
// turn whose agent calls `acme_tool__echo` through the real MCP tool gateway, and checks
// both the agent's answer and that the sandboxed panel mounted.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import { createInterface } from 'node:readline';
import { after, before, test } from 'node:test';

import { Session } from './webdriver.mjs';

const need = (name) => {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set`);
  return v;
};
const windows = process.platform === 'win32';
const work = mkdtempSync(join(windows ? tmpdir() : '/tmp', 'kk5-'));
const socket = windows ? `\\\\.\\pipe\\kumikit-e2e5-${process.pid}` : join(work, 's.sock');
const picks = join(work, 'picks');
const project = join(work, 'proj');
const bundle = need('KK_BUNDLE');

let driver;
let registry;
let indexUrl;
const answers = [];

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

  const transcript = join(work, 'm5-gateway.jsonl');
  writeFileSync(
    transcript,
    [
      { from: 'client', message: { jsonrpc: '2.0', id: 1, method: 'initialize', params: {} } },
      { from: 'agent', message: { jsonrpc: '2.0', id: 1, result: { protocolVersion: 1 } } },
      { from: 'client', message: { jsonrpc: '2.0', id: 2, method: 'session/new', params: {} } },
      { from: 'agent', message: { jsonrpc: '2.0', id: 2, result: { sessionId: 's1' } } },
      { from: 'client', message: { jsonrpc: '2.0', id: 3, method: 'session/prompt', params: {} } },
      { from: 'mcp', session: 's1', call: { name: 'acme_tool__echo', arguments: { text: 'gateway-m5-ok' } } },
      { from: 'agent', message: { jsonrpc: '2.0', id: 3, result: { stopReason: 'end_turn' } } },
    ]
      .map((l) => JSON.stringify(l))
      .join('\n') + '\n',
  );

  const toolDir = join(work, 'acme-tool');
  const agentDir = join(work, 'acme-agent');
  const panelDir = join(work, 'acme-panel');
  cpSync(join(bundle, 'test-ui-plugins/test.ui-panel'), panelDir, { recursive: true });
  const panelManifest =
    readFileSync(join(panelDir, 'plugin.toml'), 'utf8')
      .replace(
        'id = "test.ui-panel"',
        'id = "acme.panel"\ndescription = "A sandboxed panel requiring acme.tool."\nlicense = "MIT"\nrepository = "https://example.com/acme/panel"',
      )
      .replace('name = "Panel test"', 'name = "Acme Panel"')
      .replace('title = "Panel test"', 'title = "Acme Panel"') + '\n[requires]\ncore = ["acme.tool@^0.1"]\n';
  writeFileSync(join(panelDir, 'plugin.toml'), panelManifest);

  await fixture(`tool ${toolDir} 0.1.0`);
  await fixture(`agent ${agentDir} 0.1.0 ${transcript}`);
  await fixture(`publish ${toolDir}`);
  await fixture(`publish ${agentDir}`);
  await fixture(`publish ${panelDir}`);
  await fixture('starter acme.tool acme.agent acme.panel');
  await fixture('build');

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
const step = (what) => process.stderr.write(`[e2e m5] ${what}\n`);

async function palette(b, title) {
  await b.keys(['Control', 'k']);
  await b.type(await b.wait('input[aria-label="Command"]'), title);
  await b.keys('Enter');
}

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

test('registry tool plugin, gateway MCP turn and sandboxed panel on the desktop app', { timeout: 300_000 }, async () => {
  step('starting the app');
  const b = await session();
  current = b;
  try {
    await flow(b);
  } catch (e) {
    const out = process.env.KK_ARTIFACTS ?? work;
    mkdirSync(out, { recursive: true });
    await current.screenshot(join(out, 'failure-m5.png')).catch(() => {});
    step(`page text: ${await current.execute('return document.body.textContent').catch((x) => String(x))}`);
    throw e;
  }
});

async function flow(b) {
  await b.wait('footer', 'Connected');
  step('browsing starter pack');
  await b.click(await b.wait('section[aria-labelledby="kk-welcome-title"] button', 'Browse the starter pack'));
  await b.wait('section[aria-labelledby="kk-find-plugins"] li', 'acme.tool');

  step('installing acme.tool');
  await clickIn(b, 'acme.tool', 'Install on');
  await b.click(await b.wait('[role="dialog"] button', 'Grant and enable'));
  await b.until(
    async () => (await b.text(await b.wait('section[aria-labelledby="kk-core-plugins"] li', 'acme.tool'))).includes('Enabled'),
    'acme.tool enabled',
  );

  step('installing acme.agent');
  await clickIn(b, 'acme.agent', 'Install on');
  await b.click(await b.wait('[role="dialog"] button', 'Grant and enable'));
  await b.until(
    async () => (await b.text(await b.wait('section[aria-labelledby="kk-core-plugins"] li', 'acme.agent'))).includes('Enabled'),
    'acme.agent enabled',
  );

  step('installing acme.panel');
  await clickIn(b, 'acme.panel', 'Install in this app');
  await b.until(
    async () => (await b.text(await b.wait('section[aria-labelledby="kk-client-plugins"] li', 'acme.panel'))).includes('Enabled'),
    'acme.panel enabled',
  );

  step('running a gateway MCP turn');
  pick(project);
  await palette(b, 'Open a project folder');
  await b.wait('nav[aria-label="Open workspaces"] button', 'proj');
  await b.wait('select[aria-label="Agent"] option', 'Acme Agent');
  await b.type(await b.wait('#kk-chat-input'), 'use the tool');
  await b.keys('Enter');
  await b.wait('main', 'gateway-m5-ok');

  step('verifying sandboxed panel mounted');
  await b.until(
    () => b.execute('return Boolean(document.querySelector(\'iframe[title="Acme Panel"][sandbox]\'))'),
    'Acme Panel iframe present',
  );
  assert.equal(
    await b.execute('return Boolean(document.querySelector(\'iframe[title="Acme Panel"][sandbox]\'))'),
    true,
  );
  await b.end();
}
