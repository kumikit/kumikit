/**
 * The browser client and M4 (06-client, Browser client): a core serving the UI installs a
 * core plugin from its registry (the core downloads it; the browser uploads nothing), and
 * a sandboxed panel served with the bundle runs with the core's panel CSP.
 */
import { type ChildProcess, execFileSync, spawn } from 'node:child_process';
import { chmodSync, cpSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import { createInterface } from 'node:readline';

import { test as base, expect, type Page } from '@playwright/test';

import { command, openAgents, ROOT, type Registry, startRegistry } from './fixtures';

const TARGET = process.env.CARGO_TARGET_DIR ?? join(ROOT, 'target');
const exe = (name: string) => (process.platform === 'win32' ? `${name}.exe` : name);
const KUMIKIT = join(TARGET, 'debug', exe('kumikit'));

interface Served {
  url: string;
  project: string;
  registry: Registry;
  code(): string;
}

const test = base.extend<{ served: Served }>({
  ignoreHTTPSErrors: true,
  served: async ({}, use) => {
    const work = mkdtempSync(join(process.platform === 'win32' ? tmpdir() : '/tmp', 'kkr-'));
    if (process.platform !== 'win32') chmodSync(work, 0o700);
    const project = join(work, 'proj');
    mkdirSync(project);
    // The browser bundle, plus the test panel among its UI plugins.
    const web = join(work, 'web');
    cpSync(join(ROOT, 'target/bundle/ui-browser'), web, { recursive: true });
    cpSync(join(ROOT, 'target/bundle/test-ui-plugins/test.ui-panel'), join(web, 'ui-plugins', 'test.ui-panel'), { recursive: true });
    const { child: reg, registry } = await startRegistry(join(work, 'reg'));
    const socket = process.platform === 'win32' ? `\\\\.\\pipe\\kumikit-br-${process.pid}-${Math.random().toString(36).slice(2)}` : join(work, 's.sock');
    const agent = join(TARGET, 'kumikit-test', 'release');
    const child: ChildProcess = spawn(
      KUMIKIT,
      ['serve', '--socket', socket, '--data-dir', join(work, 'data'), '--listen', '127.0.0.1:0', '--web', web, '--bundled-plugins', join(ROOT, 'target/bundle/core-plugins')],
      { stdio: ['ignore', 'pipe', 'inherit'], env: { ...process.env, KUMIKIT_DEFAULT_REGISTRY: registry.url, PATH: [agent, process.env.PATH].join(delimiter) } },
    );
    const url = await new Promise<string>((ok, fail) => {
      let remote = '';
      createInterface({ input: child.stdout! }).on('line', (l) => {
        if (l.startsWith('remote ')) remote = l.split(' ')[1]!;
        if (l.startsWith('ready ')) ok(remote.replace('wss://', 'https://').replace(/\/rpc$/, '/'));
      });
      child.once('exit', (c) => fail(new Error(`the core exited (${c})`)));
    });
    await use({
      url,
      project,
      registry,
      code: () => (JSON.parse(execFileSync(KUMIKIT, ['pair', '--json', '--socket', socket], { encoding: 'utf8' })) as { code: string }).code,
    });
    child.kill();
    reg.stdin?.end();
    await new Promise((r) => (child.exitCode !== null ? r(null) : child.once('exit', r)));
    rmSync(work, { recursive: true, force: true });
  },
});

async function pair(page: Page, served: Served) {
  await page.goto(served.url);
  const form = page.getByRole('form', { name: 'Pair this browser' });
  await form.getByLabel('Pairing code').fill(served.code());
  await form.getByLabel('Name of this browser').fill('Playwright');
  await form.getByRole('button', { name: 'Pair' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
}

test('the browser client installs from the core’s registry and runs a bundled panel', async ({ page, served }) => {
  const r = served.registry;
  await r.publish(await r.probe('0.1.0'));
  await r.build();
  await pair(page, served);

  // The bundled panel runs with the core's panel CSP and the bridge.
  const out = page.frameLocator('iframe[title="Panel test"]').locator('#out[data-ready="1"]');
  await expect(out).toBeVisible({ timeout: 15_000 });
  expect(JSON.parse((await out.textContent())!)).toMatchObject({ pluginList: 'ok', send: 'permission_denied', network: 'blocked', parent: 'blocked' });

  // An agent from the registry: the core fetches it.
  const installed = await openAgents(page);
  await page.getByLabel('Search agents').fill('probe');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  const results = page.getByRole('list', { name: 'Available agents' });
  await expect(results.getByRole('listitem').filter({ hasText: 'acme.probe' })).toHaveCount(1);
  await results.getByRole('button', { name: 'Install…' }).click();
  const grant = page.getByRole('dialog', { name: /Enable Acme Probe/ });
  await expect(grant).toContainText('the publisher’s key in the registry');
  await grant.getByRole('button', { name: 'Grant and enable' }).click();
  await expect(installed.getByRole('listitem').filter({ hasText: 'acme.probe' })).toContainText('Enabled');
  await command(page, 'Manage plugins');
  // The browser cannot install UI plugins, so it offers none from the registry.
  await expect(page.getByRole('region', { name: 'On this client' }).getByRole('button', { name: 'Install from file…' })).toHaveCount(0);
});
