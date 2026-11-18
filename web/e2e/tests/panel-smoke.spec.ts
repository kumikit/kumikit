/**
 * Sandboxed panels in a browser (07-ui-plugins, level 2): a core serves
 * test-plugins/panel-smoke with `--web`; the page frames the panel, which probes its
 * sandbox and reports through the bridge's port. The same probe runs in the desktop shell
 * (scripts/desktop/panel-smoke.mjs). Nothing here needs pairing: only static files.
 */
import { type ChildProcess, spawn } from 'node:child_process';
import { chmodSync, cpSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

import { test as base, expect } from '@playwright/test';

import { ROOT } from './fixtures';

const TARGET = process.env.CARGO_TARGET_DIR ?? join(ROOT, 'target');
const KUMIKIT = join(TARGET, 'debug', process.platform === 'win32' ? 'kumikit.exe' : 'kumikit');
const SMOKE = join(ROOT, 'test-plugins', 'panel-smoke');

const test = base.extend<{ url: string }>({
  ignoreHTTPSErrors: true,
  url: async ({}, use) => {
    const work = mkdtempSync(join(process.platform === 'win32' ? tmpdir() : '/tmp', 'kkp-'));
    if (process.platform !== 'win32') chmodSync(work, 0o700);
    const web = join(work, 'web');
    cpSync(join(SMOKE, 'ui'), web, { recursive: true });
    cpSync(join(SMOKE, 'plugins'), join(web, 'ui-plugins'), { recursive: true });
    const socket = process.platform === 'win32' ? `\\\\.\\pipe\\kumikit-ps-${process.pid}-${Math.random().toString(36).slice(2)}` : join(work, 's.sock');
    const child: ChildProcess = spawn(KUMIKIT, ['serve', '--socket', socket, '--data-dir', join(work, 'data'), '--listen', '127.0.0.1:0', '--web', web], {
      stdio: ['ignore', 'pipe', 'inherit'],
    });
    const url = await new Promise<string>((ok, fail) => {
      let remote = '';
      createInterface({ input: child.stdout! }).on('line', (l) => {
        if (l.startsWith('remote ')) remote = l.split(' ')[1]!;
        if (l.startsWith('ready ')) ok(remote.replace('wss://', 'https://').replace(/\/rpc$/, '/'));
      });
      child.once('exit', (c) => fail(new Error(`the core exited (${c}); run cargo build -p kumikit-core`)));
    });
    await use(url);
    child.kill();
    rmSync(work, { recursive: true, force: true });
  },
});

test('a sandboxed panel is isolated from the page, the network and other frames', async ({ page, url }) => {
  const reports: string[] = [];
  page.on('console', (m) => {
    if (m.text().startsWith('report ')) reports.push(m.text().slice('report '.length));
  });
  const requests: string[] = [];
  page.on('request', (r) => requests.push(r.url()));
  await page.goto(url);
  await expect.poll(() => reports.find((r) => r === 'done' || r === 'timeout'), { timeout: 30_000 }).toBe('done');
  const panelLine = reports.find((r) => r.startsWith('panel '));
  expect(panelLine).toBeDefined();
  const panel = JSON.parse(panelLine!.slice('panel '.length));
  expect(panel).toMatchObject({
    classic: true,
    module: true,
    origin: 'null',
    parentDom: 'blocked',
    tauriInternals: 'undefined',
    fetchNet: 'blocked',
    fetchIpc: 'blocked',
    fetchIpcLocalhost: 'blocked',
    popup: 'blocked',
    tokens: 'rgb(1, 2, 3)',
  });
  expect(reports).not.toContain('navigated');
  // The CSP stopped the panel's fetch before it left the browser.
  expect(requests.filter((r) => r.includes('kumikit-smoke.invalid'))).toEqual([]);
  expect(page.url()).toBe(url);
});
