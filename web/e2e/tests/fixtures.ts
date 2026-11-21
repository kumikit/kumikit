/** Each test gets its own core, relay and scratch directory (no shared state). With
 * `test.use({ remote: true })` the relay also runs a second core with remote access behind
 * a cuttable network (M3). */
import { type ChildProcess, spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

import { test as base, expect, type Page } from '@playwright/test';

import { writeKkp } from './kkp';

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
  provider(transcript: string, options?: ProviderOptions): string;
  /** The same kind of provider packed as a `.kkp`, for uploading to a core elsewhere. */
  providerKkp(transcript: string, options?: ProviderOptions): string;
  /** The second core (only with `test.use({ remote: true })`). */
  remote: Remote;
  /** The fixture registry (only with `test.use({ registry: true })`). */
  registry: Registry;
}

/** A fixture registry on loopback (test-plugins/registry-fixture), publisher "acme" with a
 * key made for this test. It is the relay's, its cores' and the app's default registry. */
export interface Registry {
  /** The index URL. */
  url: string;
  /** Writes the probe test plugin as `acme.probe` at `version`; returns its folder.
   * `extra` are more `[permissions]` lines, joined with `;`. */
  probe(version: string, extra?: string): Promise<string>;
  /** Writes the tool test plugin as `acme.tool` at `version`; returns its folder. */
  tool(version: string): Promise<string>;
  /** Writes a declarative ACP agent as `acme.agent` at `version` replaying `transcript`;
   * returns its folder. */
  agent(version: string, transcript: string): Promise<string>;
  publish(dir: string, how?: 'signed' | 'unsigned' | 'other-key'): Promise<void>;
  starter(...ids: string[]): Promise<void>;
  /** Writes index.json. */
  build(): Promise<void>;
  /** Requests the registry has served so far. */
  hits(): Promise<number>;
}

/** Starts the fixture registry; returns it and its process. */
export async function startRegistry(root: string): Promise<{ child: ChildProcess; registry: Registry }> {
  const exe = join(TARGET, 'debug', process.platform === 'win32' ? 'registry-fixture.exe' : 'registry-fixture');
  const child = spawn(exe, [root], { stdio: ['pipe', 'pipe', 'inherit'] });
  const lines = createInterface({ input: child.stdout! });
  const queue: ((line: string) => void)[] = [];
  const url = await new Promise<string>((ok, fail) => {
    lines.once('line', (l) => (l.startsWith('ready ') ? ok(l.slice(6)) : fail(new Error(l))));
    child.once('exit', (code) => fail(new Error(`the registry fixture exited (${code}); run cargo build -p kumikit-test-registry-fixture`)));
  });
  lines.on('line', (l) => queue.shift()?.(l));
  const send = (command: string) =>
    new Promise<string>((ok, fail) => {
      queue.push((l) => (l.startsWith('ok') ? ok(l.slice(2).trim()) : fail(new Error(`${command}: ${l}`))));
      child.stdin!.write(`${command}\n`);
    });
  let n = 0;
  return {
    child,
    registry: {
      url,
      async probe(version, extra = '') {
        const dir = join(root, `probe-${version}-${++n}`);
        await send(`probe ${dir} ${version} ${extra}`.trim());
        return dir;
      },
      async tool(version) {
        const dir = join(root, `tool-${version}-${++n}`);
        await send(`tool ${dir} ${version}`);
        return dir;
      },
      async agent(version, transcript) {
        const dir = join(root, `agent-${version}-${++n}`);
        await send(`agent ${dir} ${version} ${transcript}`);
        return dir;
      },
      publish: async (dir, how = 'signed') => void (await send(`${how === 'signed' ? 'publish' : how === 'unsigned' ? 'publish-unsigned' : 'publish-other-key'} ${dir}`)),
      starter: async (...ids) => void (await send(`starter ${ids.join(' ')}`)),
      build: async () => void (await send('build')),
      hits: async () => Number(await send('hits')),
    },
  };
}

export interface ProviderOptions {
  mode?: string;
  live?: boolean;
}

/** The relay's second core, with remote access behind a cuttable network. */
export interface Remote {
  /** A project folder on the remote core's machine (the same machine, in tests). */
  project: string;
  /** A pairing code from `kumikit pair`, with the address clients use (the proxy's). */
  pairCode(): Promise<{ code: string; fingerprint: string; address: string }>;
  /** Cuts the remote's network: a black hole, nothing closes. */
  cut(): Promise<void>;
  /** Restores it: connections from before the cut close. */
  restore(): Promise<void>;
  /** Revokes every device paired with the remote core. */
  revoke(): Promise<void>;
  /** Restarts the remote core with a new certificate. */
  rotate(): Promise<void>;
}

function startRelay(work: string, remote: boolean, registry?: string): Promise<{ child: ChildProcess; url: string }> {
  const exe = join(TARGET, 'debug', process.platform === 'win32' ? 'kumikit-web-relay.exe' : 'kumikit-web-relay');
  const args = ['--ui', join(BUNDLE, 'ui-browser-test'), '--ui-plugins', join(BUNDLE, 'ui-plugins'), '--core-plugins', join(BUNDLE, 'core-plugins'), '--work', work];
  if (remote) args.push('--remote');
  const env = registry ? { ...process.env, KUMIKIT_DEFAULT_REGISTRY: registry } : process.env;
  const child = spawn(exe, args, { stdio: ['pipe', 'pipe', 'inherit'], env });
  return new Promise((ok, fail) => {
    const lines = createInterface({ input: child.stdout! });
    lines.once('line', (l) => (l.startsWith('ready ') ? ok({ child, url: l.slice(6) }) : fail(new Error(l))));
    child.once('exit', (code) => fail(new Error(`the relay exited (${code}); run node scripts/bundle.mjs and build kumikit-web-relay`)));
  });
}

/** A declarative provider's manifest: `fake-acp-agent` replaying a transcript, or (live)
 * the real Claude Code ACP adapter. */
function manifest(n: number, transcript: string, options: ProviderOptions): string {
  const mode = options.mode ? `, mode = "${options.mode}"` : '';
  const file = join(TRANSCRIPTS, `${transcript}.jsonl`).replaceAll('\\', '\\\\');
  const [program, args] = options.live ? ['claude-agent-acp', ''] : ['fake-acp-agent', `, args = ["replay", "${file}"]`];
  return [
    `id = "test.recorded${n}"`,
    `name = "Recorded ${transcript}"`,
    'version = "0.1.0"',
    'kind = ["provider"]',
    'api = "^0.2"',
    '[permissions]',
    `spawn = ["${program}"]`,
    '[contributes]',
    `providers = [{ id = "agent", name = "Recorded agent ${n}", acp = { command = "${program}"${args}${mode} } }]`,
    '',
  ].join('\n');
}

export const test = base.extend<{ kk: Kumikit; remote: boolean; registry: boolean }>({
  remote: [false, { option: true }],
  registry: [false, { option: true }],
  kk: async ({ remote, registry: withRegistry }, use) => {
    // Short: Unix socket paths are limited to about 104 bytes.
    const work = mkdtempSync(join(process.platform === 'win32' ? tmpdir() : '/tmp', 'kke-'));
    const project = join(work, 'proj');
    mkdirSync(project);
    const remoteProject = join(work, 'rproj');
    mkdirSync(remoteProject);
    const fixture = withRegistry ? await startRegistry(join(work, 'reg')) : undefined;
    const { child, url } = await startRelay(join(work, 'w'), remote, fixture?.registry.url);
    const send = (path: string, body?: unknown) =>
      fetch(url + path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: body === undefined ? null : JSON.stringify(body) });
    const post = async (path: string, body?: unknown) => {
      const r = await send(path, body);
      if (!r.ok) throw new Error(`${path}: ${r.status} ${await r.text()}`);
    };
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
        writeFileSync(join(dir, 'plugin.toml'), manifest(n, transcript, options));
        return dir;
      },
      providerKkp(transcript, options = {}) {
        ++n;
        return writeKkp(join(work, `provider-${n}.kkp`), { 'plugin.toml': manifest(n, transcript, options) });
      },
      remote: {
        project: remoteProject,
        async pairCode() {
          const r = await send('/test/pair-code');
          if (!r.ok) throw new Error(`pair-code: ${r.status} ${await r.text()}`);
          return (await r.json()) as { code: string; fingerprint: string; address: string };
        },
        cut: () => post('/test/cut'),
        restore: () => post('/test/restore'),
        revoke: () => post('/test/revoke'),
        rotate: () => post('/test/rotate'),
      },
      get registry(): Registry {
        if (!fixture) throw new Error('use test.use({ registry: true }) for a registry');
        return fixture.registry;
      },
    });
    // Closing stdin stops the relay and its cores, and the registry.
    child.stdin?.end();
    await new Promise((r) => (child.exitCode !== null ? r(null) : child.once('exit', r)));
    if (fixture) {
      fixture.child.stdin?.end();
      await new Promise((r) => (fixture.child.exitCode !== null ? r(null) : fixture.child.once('exit', r)));
    }
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
  const input = page.getByRole('dialog', { name: 'Commands' }).getByLabel('Command', { exact: true });
  await input.fill(title);
  await input.press('Enter');
}
