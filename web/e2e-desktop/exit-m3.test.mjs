// M3's exit test on the real desktop app (12-roadmap): one window shows tabs from a local
// core and a remote core; killing the remote's network shows "reconnecting" only in its
// tabs, and the local tabs keep streaming.
//
// The app starts the local core as usual. The test starts the "remote" core itself, with
// remote access on loopback, behind `net-cut` (test-plugins/net-cut), a proxy that becomes a
// black hole on `cut`, which is what a lost network looks like. The app pairs with it
// through the shell's real code: pinned TLS, a device token in the client secrets (a 0600
// file on Linux, Credential Manager on Windows). Agents replay a recorded Claude Code
// transcript (fake-acp-agent).
//
// Needs, in the environment (scripts/desktop/e2e.sh sets them up on Linux; CI on
// Windows): KK_APP, KK_CORE_BIN, KK_FAKE_AGENT_DIR, KK_NET_CUT, KK_BUNDLE, and tauri-driver
// on PATH.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { delimiter, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { Session } from './webdriver.mjs';

const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';
const ROOT = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const need = (name) => {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set; see the comment at the top of this file`);
  return v;
};
const windows = process.platform === 'win32';
const work = mkdtempSync(join(windows ? tmpdir() : '/tmp', 'kkm-'));
const socket = windows ? `\\\\.\\pipe\\kumikit-e2e3-${process.pid}` : join(work, 's.sock');
const remoteSocket = windows ? `\\\\.\\pipe\\kumikit-e2e3r-${process.pid}` : join(work, 'r.sock');
const picks = join(work, 'picks');
const project = join(work, 'proj');
const remoteProject = join(work, 'rproj');
const bundle = need('KK_BUNDLE');
const core = need('KK_CORE_BIN');
const PATH = [need('KK_FAKE_AGENT_DIR'), process.env.PATH].join(delimiter);
const step = (what) => process.stderr.write(`[e2e m3] ${what}\n`);

let driver;
let remote;
let cut;
let cutLines;

/** Starts a child and waits for a stdout line that `match` accepts; returns the line. */
function started(child, match, what) {
  return new Promise((ok, fail) => {
    const lines = createInterface({ input: child.stdout });
    lines.on('line', (l) => {
      if (match(l)) ok({ line: l, lines });
    });
    child.once('exit', (code) => fail(new Error(`${what} exited (${code})`)));
  });
}

/** A declarative provider replaying a transcript, packed and installed with the CLI. */
/** Waits until a core has seeded and enabled provider-acp, which the recorded providers
 * run on (bundled plugins are seeded after the core is ready). */
async function providerAcpReady(onSocket) {
  for (let i = 0; i < 600; i++) {
    const out = execFileSync(core, ['plugin', 'list', '--socket', onSocket], { encoding: 'utf8' });
    if (/kumikit\.provider-acp \S+\s+enabled/.test(out)) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('provider-acp was not seeded');
}

function installProvider(onSocket, name) {
  const dir = join(work, name);
  mkdirSync(dir, { recursive: true });
  const transcript = join(ROOT, 'plugins/provider-acp/tests/transcripts/claude-tool-allow.jsonl').replaceAll('\\', '\\\\');
  writeFileSync(
    join(dir, 'plugin.toml'),
    [
      `id = "test.${name}"`,
      `name = "Recorded ${name}"`,
      'version = "0.1.0"',
      'kind = ["provider"]',
      'api = "^0.2"',
      '[permissions]',
      'spawn = ["fake-acp-agent"]',
      '[contributes]',
      `providers = [{ id = "claude", name = "Recorded ${name}", acp = { command = "fake-acp-agent", args = ["replay", "${transcript}"], mode = "default" } }]`,
      '',
    ].join('\n'),
  );
  const kkp = join(work, `${name}.kkp`);
  execFileSync(core, ['plugin', 'pack', dir, '-o', kkp]);
  execFileSync(core, ['plugin', 'add', kkp, '--yes', '--socket', onSocket], { env: { ...process.env, PATH } });
}

/** Sends a command to net-cut and waits for its `ok`. */
function network(cmd) {
  return new Promise((ok) => {
    cutLines.once('line', () => ok());
    cut.stdin.write(`${cmd}\n`);
  });
}

before(async () => {
  mkdirSync(project);
  mkdirSync(remoteProject);
  writeFileSync(picks, '');

  // The remote core, and the cuttable network in front of it.
  remote = spawn(
    core,
    [
      'serve', '--socket', remoteSocket, '--data-dir', join(work, 'remote-core'), '--bundled-plugins', join(bundle, 'core-plugins'),
      '--listen', '127.0.0.1:0', '--keepalive-ping-ms', '200', '--keepalive-idle-ms', '1500',
    ],
    { env: { ...process.env, PATH }, stdio: ['ignore', 'pipe', 'inherit'] },
  );
  let address;
  const r = createInterface({ input: remote.stdout });
  await new Promise((ok, fail) => {
    r.on('line', (l) => {
      if (l.startsWith('remote ')) address = l.split(' ')[1].replace('wss://', '').replace('/rpc', '');
      if (l.startsWith('ready ')) ok();
    });
    remote.once('exit', (c) => fail(new Error(`the remote core exited (${c})`)));
  });
  cut = spawn(need('KK_NET_CUT'), ['--to', address], { stdio: ['pipe', 'pipe', 'inherit'] });
  const { line, lines } = await started(cut, (l) => l.startsWith('ready '), 'net-cut');
  cutLines = lines;
  process.env.KK_PROXY = line.slice(6);
  installProvider(remoteSocket, 'remote');
  await providerAcpReady(remoteSocket);

  const env = {
    ...process.env,
    PATH,
    KUMIKIT_SOCKET: socket,
    KUMIKIT_CORE_DATA_DIR: join(work, 'core'),
    KUMIKIT_CLIENT_DATA_DIR: join(work, 'client'),
    KUMIKIT_CORE_BIN: core,
    KUMIKIT_UI_DIR: join(bundle, 'ui'),
    KUMIKIT_BUNDLED_UI_PLUGINS: join(bundle, 'ui-plugins'),
    KUMIKIT_BUNDLED_CORE_PLUGINS: join(bundle, 'core-plugins'),
    KUMIKIT_TEST_PICKS: picks,
    // A lost network is noticed in seconds (test builds only).
    KUMIKIT_TEST_KEEPALIVE: '200,1500',
    WEBKIT_DISABLE_DMABUF_RENDERER: '1',
  };
  driver = spawn('tauri-driver', [], { env, stdio: 'inherit' });
  for (let i = 0; i < 200; i++) {
    const up = await new Promise((done) => net.connect(4444, '127.0.0.1').once('connect', function () { this.end(); done(true); }).once('error', () => done(false)));
    if (up) return;
    await new Promise((done) => setTimeout(done, 50));
  }
  throw new Error('tauri-driver did not start');
});

after(async () => {
  // Stop the core the app started (it outlives the window by design).
  await new Promise((done) => {
    const c = net.connect(socket);
    c.once('error', () => done());
    c.once('connect', () => {
      c.write('{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{"protocol":"0.10","client":{"name":"e2e","version":"0"}}}\n');
      c.write('{"jsonrpc":"2.0","id":2,"method":"core.shutdown","params":{}}\n');
    });
    c.once('close', () => done());
    c.resume();
  });
  remote?.kill();
  cut?.kill();
  driver?.kill();
  rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
});

const pick = (path) => writeFileSync(picks, path + '\n');

async function palette(b, title) {
  await b.keys(['Control', 'k']);
  await b.type(await b.wait('input[aria-label="Command"]'), title);
  await b.keys('Enter');
}

const button = (b, scope, label) => b.wait(`${scope} button`, label);

/** The input inside the label whose text contains `text`. */
async function field(b, text) {
  await b.wait('label', text);
  const el = await b.execute(
    'return [...document.querySelectorAll("label")].find((l) => l.textContent.includes(arguments[0]))?.querySelector("input") ?? null',
    [text],
  );
  return el[ELEMENT];
}

const TABS = 'nav[aria-label="Open workspaces"]';
const tabText = async (b, core) => b.text(await b.wait(`${TABS} [data-core="${core}"]`));
const APPROVAL = 'section[aria-label="Approval"]';

let current;

async function report(b) {
  const out = process.env.KK_ARTIFACTS ?? work;
  mkdirSync(out, { recursive: true });
  await b.screenshot(join(out, 'failure-m3.png')).catch(() => {});
  step(`page text: ${await b.execute('return document.body.textContent').catch((x) => String(x))}`);
}

test('the M3 exit criteria on the desktop app', { timeout: 300_000 }, async () => {
  const b = await Session.start('http://127.0.0.1:4444', { 'tauri:options': { application: need('KK_APP') } });
  current = b;
  try {
    await flow(b);
  } catch (e) {
    await report(current);
    throw e;
  } finally {
    await b.end().catch(() => {});
  }
});

async function flow(b) {
  await b.wait('footer', 'Connected');
  // The local core is the one the app started; give it the same recorded provider.
  installProvider(socket, 'local');
  await providerAcpReady(socket);

  step('adding the remote core');
  const printed = JSON.parse(execFileSync(core, ['pair', '--json', '--socket', remoteSocket], { encoding: 'utf8' }));
  await palette(b, 'Cores: add, remove, reconnect');
  await b.click(await button(b, 'main', 'With a pairing code'));
  await b.type(await field(b, 'Address'), process.env.KK_PROXY);
  await b.type(await field(b, 'Pairing code'), printed.code);
  await b.type(await field(b, 'Name'), 'Dev box');
  await b.click(await button(b, 'form', 'Check the core'));
  // The fingerprint is shown before the code is sent; it is the one `kumikit pair` printed.
  const shown = await b.text(await b.wait('[data-fingerprint]'));
  assert.equal(shown.trim(), printed.fingerprint);
  await b.click(await button(b, 'main', 'It matches: pair'));
  await b.wait('ul[aria-label="Cores"] li', 'Connected');
  await b.until(async () => (await b.text(await b.wait('ul[aria-label="Cores"] li', 'Dev box'))).includes('Connected'), 'the remote core to connect');
  await b.click(await b.wait('[data-overlay="cores"] button[aria-label="Close"]'));

  step('a thread on the remote core, waiting on an approval');
  await b.click(await b.wait('section[aria-label="Dev box"] button[aria-label^="Open folder"]'));
  await b.type(await field(b, 'Path on'), remoteProject);
  await b.click(await button(b, '[role="dialog"]', 'Open'));
  await b.wait(`${TABS} [data-core="dev-box"]`);
  await b.wait('select[aria-label="Agent"] option');
  await b.click(await b.wait('button[aria-label^="Permissions:"]'));
  await b.click(await b.wait('[role="menuitemradio"]', 'Standard'));
  await b.type(await b.wait('#kk-chat-input'), 'Add a line to notes.txt');
  await b.keys('Enter');
  await b.wait(APPROVAL, 'Needs your approval');

  step('a tab on the local core');
  pick(project);
  await b.click(await b.wait('section[aria-label="This computer"] button[aria-label^="Open folder"]'));
  await b.wait(`${TABS} [data-core="local"]`);

  step('cutting the remote network');
  await network('cut');
  await b.until(async () => (await tabText(b, 'dev-box')).includes('reconnecting'), 'the remote tab to say reconnecting');
  assert.ok(!(await tabText(b, 'local')).includes('reconnecting'), 'the local tab is not reconnecting');

  step('the local core keeps streaming');
  await b.wait('select[aria-label="Agent"] option');
  await b.click(await b.wait('button[aria-label^="Permissions:"]'));
  await b.click(await b.wait('[role="menuitemradio"]', 'Standard'));
  await b.type(await b.wait('#kk-chat-input'), 'Add a line to notes.txt');
  await b.keys('Enter');
  await b.wait(APPROVAL, 'Needs your approval');
  await b.click(await button(b, APPROVAL, 'Allow'));
  await b.wait(APPROVAL, 'Allowed');
  await b.wait('figcaption', 'Changed');
  assert.ok((await tabText(b, 'dev-box')).includes('reconnecting'), 'the remote tab is still reconnecting');
  assert.ok(!(await tabText(b, 'local')).includes('reconnecting'), 'the local tab is still not reconnecting');

  step('restoring the network');
  await network('restore');
  await b.click(await b.wait(`${TABS} [data-core="dev-box"] button`));
  const now = await b.find('main button', 'Reconnect now');
  if (now) await b.click(now).catch(() => {});
  await b.until(async () => !(await tabText(b, 'dev-box')).includes('reconnecting'), 'the remote tab to reconnect');
  await b.wait(APPROVAL, 'Needs your approval');
  await b.click(await button(b, APPROVAL, 'Allow'));
  await b.wait(APPROVAL, 'Allowed');
  await b.wait('figcaption', 'Changed');
}
