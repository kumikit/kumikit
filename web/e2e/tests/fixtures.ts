/** Each test gets its own core, relay and scratch directory (no shared state). */
import { type ChildProcess, spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

import { test as base, expect, type Page } from '@playwright/test';

export const ROOT = resolve(fileURLToPath(new URL('../../..', import.meta.url)));
const TARGET = process.env.CARGO_TARGET_DIR ?? join(ROOT, 'target');
const BUNDLE = join(ROOT, 'target/bundle');
export const TRANSCRIPTS = join(ROOT, 'plugins/provider-acp/tests/transcripts');

export interface Kumikit {
  url: string;
  /** Scratch directory of this test. */
  work: string;
  /** A project folder for `workspace.open`. */
  project: string;
  /** What the next file or folder picker returns. */
  pick(path: string | null): Promise<void>;
  /** Drops every bridge connection, as a lost network would. */
  drop(): Promise<void>;
  /** A declarative ACP provider replaying a recorded transcript; returns its folder. */
  provider(transcript: string, options?: { mode?: string; live?: boolean }): string;
}

function startRelay(work: string): Promise<{ child: ChildProcess; url: string }> {
  const exe = join(TARGET, 'debug', process.platform === 'win32' ? 'kumikit-web-relay.exe' : 'kumikit-web-relay');
  const child = spawn(
    exe,
    ['--ui', join(BUNDLE, 'ui-browser-test'), '--ui-plugins', join(BUNDLE, 'ui-plugins'), '--core-plugins', join(BUNDLE, 'core-plugins'), '--work', work],
    { stdio: ['pipe', 'pipe', 'inherit'] },
  );
  return new Promise((ok, fail) => {
    const lines = createInterface({ input: child.stdout! });
    lines.once('line', (l) => (l.startsWith('ready ') ? ok({ child, url: l.slice(6) }) : fail(new Error(l))));
    child.once('exit', (code) => fail(new Error(`the relay exited (${code}); run node scripts/bundle.mjs and build kumikit-web-relay`)));
  });
}

export const test = base.extend<{ kk: Kumikit }>({
  kk: async ({}, use) => {
    // Short: Unix socket paths are limited to about 104 bytes.
    const work = mkdtempSync(join(process.platform === 'win32' ? tmpdir() : '/tmp', 'kke-'));
    const project = join(work, 'proj');
    mkdirSync(project);
    const { child, url } = await startRelay(join(work, 'w'));
    const post = (path: string, body?: unknown) =>
      fetch(url + path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: body === undefined ? null : JSON.stringify(body) }).then(() => {});
    let n = 0;
    await use({
      url,
      work,
      project,
      pick: (path) => post('/test/pick', { path }),
      drop: () => post('/test/drop'),
      provider(transcript, options = {}) {
        const dir = join(work, `provider-${++n}`);
        mkdirSync(dir);
        const mode = options.mode ? `, mode = "${options.mode}"` : '';
        const file = join(TRANSCRIPTS, `${transcript}.jsonl`).replaceAll('\\', '\\\\');
        // Live: the real Claude Code ACP adapter instead of a recording.
        const [program, args] = options.live ? ['claude-agent-acp', ''] : ['fake-acp-agent', `, args = ["replay", "${file}"]`];
        writeFileSync(
          join(dir, 'plugin.toml'),
          `id = "test.recorded${n}"
name = "Recorded ${transcript}"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
[permissions]
spawn = ["${program}"]
[contributes]
providers = [{ id = "agent", name = "Recorded agent ${n}", acp = { command = "${program}"${args}${mode} } }]
`,
        );
        return dir;
      },
    });
    // Closing stdin stops the relay and its core.
    child.stdin?.end();
    await new Promise((r) => (child.exitCode !== null ? r(null) : child.once('exit', r)));
    rmSync(work, { recursive: true, force: true });
  },
});

export { expect };

/** Opens the app and waits until it is connected. */
export async function open(page: Page, kk: Kumikit, query = ''): Promise<void> {
  await page.goto(kk.url + '/index.html' + query);
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
}

/** `Mod+<key>` as the UI kernel reads it: Cmd when the browser says it is on macOS
 * (Playwright's Safari device does, on any host), Ctrl otherwise. */
export async function mod(page: Page, key: string): Promise<void> {
  const mac = await page.evaluate(() => navigator.platform.startsWith('Mac') || navigator.userAgent.includes('Mac OS'));
  await page.keyboard.press(`${mac ? 'Meta' : 'Control'}+${key}`);
}

/** Runs a command from the palette, as a keyboard user would. */
export async function command(page: Page, title: string): Promise<void> {
  await mod(page, 'k');
  const input = page.getByRole('combobox').or(page.getByLabel('Command', { exact: true }));
  await input.fill(title);
  await input.press('Enter');
}
