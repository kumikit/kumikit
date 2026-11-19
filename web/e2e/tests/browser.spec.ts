/**
 * The browser client (06-client, Browser client): a core serves the UI over HTTPS
 * (`kumikit serve --listen … --web target/bundle/ui-browser`) and a plain browser pairs with
 * it from the page. No relay: the page talks to the core itself. The agent replays a
 * recorded Claude Code transcript.
 */
import { type ChildProcess, execFileSync, spawn } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import { createInterface } from 'node:readline';

import AxeBuilder from '@axe-core/playwright';
import { test as base, expect, type Page } from '@playwright/test';

import { command, ROOT } from './fixtures';
import { writeKkp } from './kkp';

const TARGET = process.env.CARGO_TARGET_DIR ?? join(ROOT, 'target');
const exe = (name: string) => (process.platform === 'win32' ? `${name}.exe` : name);
const KUMIKIT = join(TARGET, 'debug', exe('kumikit'));

/** Where the test build put the fake agent (test-plugins/kumikit-test-plugins). */
function agentDir(): string {
  const dir = join(TARGET, 'kumikit-test', 'release');
  if (!existsSync(join(dir, exe('fake-acp-agent')))) throw new Error('build the test helpers: cargo build -p kumikit-test-plugins');
  return dir;
}

interface WebCore {
  url: string;
  work: string;
  project: string;
  socket: string;
  code(): { code: string; fingerprint: string };
}

/** Writes the page's WebSocket events and console problems to the test output, next to
 * the core's debug log, so a CI failure shows both sides of the connection. */
function watch(page: Page): void {
  const at = () => new Date().toISOString();
  page.on('websocket', (ws) => {
    console.log(`${at()} PAGE websocket open ${ws.url()}`);
    ws.on('socketerror', (e) => console.log(`${at()} PAGE websocket error ${e}`));
    ws.on('close', () => console.log(`${at()} PAGE websocket closed`));
  });
  page.on('console', (m) => {
    if (m.type() === 'error' || m.type() === 'warning') console.log(`${at()} PAGE console ${m.type()}: ${m.text()}`);
  });
  page.on('pageerror', (e) => console.log(`${at()} PAGE error: ${e.message}`));
}

const test = base.extend<{ core: WebCore }>({
  // The core's certificate is self-signed: the browser is told to accept it, as a user
  // would after comparing the fingerprint.
  ignoreHTTPSErrors: true,
  page: async ({ page }, use) => {
    watch(page);
    await use(page);
  },
  core: async ({}, use) => {
    const work = mkdtempSync(join(process.platform === 'win32' ? tmpdir() : '/tmp', 'kkb-'));
    if (process.platform !== 'win32') chmodSync(work, 0o700);
    const project = join(work, 'proj');
    mkdirSync(project);
    const socket = process.platform === 'win32' ? `\\\\.\\pipe\\kumikit-browser-${process.pid}-${Math.random().toString(36).slice(2)}` : join(work, 's.sock');
    const child: ChildProcess = spawn(
      KUMIKIT,
      ['serve', '--socket', socket, '--data-dir', join(work, 'data'), '--listen', '127.0.0.1:0', '--web', join(ROOT, 'target/bundle/ui-browser'), '--bundled-plugins', join(ROOT, 'target/bundle/core-plugins')],
      // Debug logs of the core's connections go to the test output, for CI failures.
      { stdio: ['ignore', 'pipe', 'inherit'], env: { ...process.env, KUMIKIT_LOG: 'kumikit_core=debug', PATH: [agentDir(), process.env.PATH].join(delimiter) } },
    );
    const url = await new Promise<string>((ok, fail) => {
      const lines = createInterface({ input: child.stdout! });
      let remote = '';
      lines.on('line', (l) => {
        if (l.startsWith('remote ')) remote = l.split(' ')[1]!;
        if (l.startsWith('ready ')) ok(remote.replace('wss://', 'https://').replace(/\/rpc$/, '/'));
      });
      child.once('exit', (c) => fail(new Error(`the core exited (${c}); run node scripts/bundle.mjs and cargo build -p kumikit-core`)));
    });
    await use({
      url,
      work,
      project,
      socket,
      code: () => JSON.parse(execFileSync(KUMIKIT, ['pair', '--json', '--socket', socket], { encoding: 'utf8' })) as { code: string; fingerprint: string },
    });
    child.kill();
    await new Promise((r) => (child.exitCode !== null ? r(null) : child.once('exit', r)));
    rmSync(work, { recursive: true, force: true });
  },
});

async function pair(page: Page, core: WebCore) {
  await page.goto(core.url);
  const form = page.getByRole('form', { name: 'Pair this browser' });
  await expect(form).toBeVisible();
  await form.getByLabel('Pairing code').fill(core.code().code);
  await form.getByLabel('Name of this browser').fill('Playwright');
  await form.getByRole('button', { name: 'Pair' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
}

test('pair a browser, then the M2 flow through the core it came from', async ({ page, core, context }) => {
  await pair(page, core);

  // The token is in a cookie the page cannot read.
  expect(await page.evaluate(() => document.cookie)).not.toContain('kk_device');
  const cookie = (await context.cookies()).find((c) => c.name === 'kk_device');
  expect(cookie).toMatchObject({ httpOnly: true, secure: true, sameSite: 'Strict' });

  // A provider is installed by upload: the browser cannot reach the core's files.
  await command(page, 'Manage plugins');
  const region = page.getByRole('region', { name: /On this core/ });
  await expect(region.getByRole('button', { name: 'Install from folder…' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'On this client' }).getByRole('button', { name: 'Install from file…' })).toHaveCount(0);
  const transcript = join(ROOT, 'plugins/provider-acp/tests/transcripts/claude-tool-allow.jsonl').replaceAll('\\', '\\\\');
  const kkp = writeKkp(join(core.work, 'p.kkp'), {
    'plugin.toml': [
      'id = "test.recorded"',
      'name = "Recorded Claude Code"',
      'version = "0.1.0"',
      'kind = ["provider"]',
      'api = "^0.2"',
      '[permissions]',
      'spawn = ["fake-acp-agent"]',
      '[contributes]',
      `providers = [{ id = "claude", name = "Recorded Claude Code", acp = { command = "fake-acp-agent", args = ["replay", "${transcript}"], mode = "default" } }]`,
      '',
    ].join('\n'),
  });
  const chooser = page.waitForEvent('filechooser');
  await region.getByRole('button', { name: 'Install from file…' }).click();
  await (await chooser).setFiles(kkp);
  await page.getByRole('dialog', { name: /Enable Recorded Claude Code/ }).getByRole('button', { name: 'Grant and enable' }).click();
  await expect(region.getByRole('listitem').filter({ hasText: 'test.recorded' })).toContainText('Enabled');
  // The recorded provider runs on provider-acp, which the core seeds after it is ready.
  await expect(region.getByRole('listitem').filter({ hasText: 'kumikit.provider-acp' })).toContainText('Enabled');

  // A folder on the core's machine is typed.
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  const open = page.getByRole('dialog', { name: /Open a project folder/ });
  await open.getByLabel(/Path on/).fill(core.project);
  await open.getByRole('button', { name: 'Open' }).click();
  // "New thread" needs an open tab; pressing it from the palette before the tab exists
  // matches nothing (this raced on the macOS runner).
  await expect(page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true })).toBeVisible();
  await command(page, 'New thread');
  const dialog = page.getByRole('dialog', { name: 'New thread' });
  await dialog.getByLabel('Agent').selectOption({ label: 'Recorded Claude Code' });
  await dialog.getByRole('button', { name: 'Start' }).click();
  await page.getByLabel('Message').fill('Add a line to notes.txt');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Needs your approval');

  // Reloading the page keeps the pairing and resumes the waiting turn.
  await page.reload();
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Needs your approval');
  await page.getByRole('region', { name: 'Approval' }).getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page.getByText(/Changed/).first()).toBeVisible();

  const panel = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--kk-surface-panel').trim());
  await command(page, 'Theme: Kumikit Dark');
  await expect.poll(panel).toBe('#15171c');

  // Signing out revokes this browser: the pairing screen comes back.
  await command(page, 'Sign out of this core');
  await expect(page.getByRole('form', { name: 'Pair this browser' })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('form', { name: 'Pair this browser' })).toBeVisible();
});

test('a wrong code is refused, and the pairing screen has no axe violations', async ({ page, core }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto(core.url);
  const form = page.getByRole('form', { name: 'Pair this browser' });
  await expect(form).toBeVisible();
  const r = await new AxeBuilder({ page: page as never }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)).toEqual([]);
  await form.getByLabel('Pairing code').fill('0000-0000');
  await form.getByRole('button', { name: 'Pair' }).click();
  await expect(page.getByRole('alert')).toContainText('wrong, used or expired');
  // The real code still works: one wrong try does not burn it.
  await pair(page, core);
});

test('the core serves the favicon the page links to, before pairing', async ({ page, core }) => {
  await page.goto(core.url);
  const href = await page.locator('link[rel="icon"]').getAttribute('href');
  expect(href).toBe('./favicon.svg');
  const r = await page.request.get(new URL(href!, core.url).href);
  expect(r.status()).toBe(200);
  expect(r.headers()['content-type']).toBe('image/svg+xml');
  expect(await r.text()).toContain('<title>Kumikit</title>');
});
