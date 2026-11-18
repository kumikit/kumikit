// Sandboxed panels in the real desktop shell (07-ui-plugins, level 2), on any webview and
// without WebDriver: runs a test-hooks build of the shell with test-plugins/panel-smoke as
// its UI and bundled UI plugins, reads the reports it prints, and checks them.
//
// Usage: node scripts/desktop/panel-smoke.mjs <kumikit-desktop built with --features test-hooks>
// On Linux run it under a display (xvfb-run, or the desktop container).
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const app = process.argv[2];
if (!app) {
  console.error('usage: node scripts/desktop/panel-smoke.mjs <kumikit-desktop>');
  process.exit(2);
}
const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const smoke = join(root, 'test-plugins', 'panel-smoke');
const windows = process.platform === 'win32';
const work = mkdtempSync(join(windows ? tmpdir() : '/tmp', 'kkps-'));
const env = {
  ...process.env,
  KUMIKIT_UI_DIR: join(smoke, 'ui'),
  KUMIKIT_BUNDLED_UI_PLUGINS: join(smoke, 'plugins'),
  KUMIKIT_CLIENT_DATA_DIR: join(work, 'client'),
  KUMIKIT_CORE_DATA_DIR: join(work, 'core'),
  // The page never connects, so no core starts; these keep a mistake away from real data.
  KUMIKIT_SOCKET: windows ? `\\\\.\\pipe\\kumikit-ps-${process.pid}` : join(work, 's.sock'),
  KUMIKIT_CORE_BIN: join(work, 'no-core'),
  WEBKIT_DISABLE_DMABUF_RENDERER: '1',
};

const child = spawn(app, [], { env, stdio: ['ignore', 'pipe', 'inherit'] });
const lines = [];
let panel = null;
const finished = new Promise((done) => {
  createInterface({ input: child.stdout }).on('line', (line) => {
    console.log(`app: ${line}`);
    lines.push(line);
    if (line.startsWith('report panel ')) panel = JSON.parse(line.slice('report panel '.length));
    if (line === 'report done' || line === 'report timeout') done(line);
  });
  child.on('exit', () => done('exited'));
});
const deadline = new Promise((done) => setTimeout(() => done('deadline'), 90_000));
const how = await Promise.race([finished, deadline]);
child.kill();
await new Promise((done) => (child.exitCode !== null ? done() : child.once('exit', done)));

const problems = [];
if (how !== 'report done') problems.push(`the panel did not finish (${how})`);
if (lines.some((l) => l.includes('FORGED'))) problems.push('a forged IPC call from the panel reached the shell');
if (lines.includes('report navigated')) problems.push('the panel navigated');
if (panel) {
  const expect = {
    classic: true,
    parentDom: 'blocked',
    fetchIpc: 'blocked',
    fetchIpcLocalhost: 'blocked',
    fetchNet: 'blocked',
    popup: 'blocked',
    tokens: 'rgb(1, 2, 3)',
  };
  // Shell commands must stay out of reach, whether or not Tauri's script is in the frame.
  for (const key of ['invokeReport', 'invokeProfiles']) {
    if (String(panel[key]).startsWith('ANSWERED')) problems.push(`${key}: a panel reached a shell command (${panel[key]})`);
  }
  console.log(`Tauri script in the frame: ${panel.tauriInternals}; invoke: ${panel.invokeReport} / ${panel.invokeProfiles}`);
  for (const [key, want] of Object.entries(expect)) {
    if (panel[key] !== want) problems.push(`${key}: expected ${JSON.stringify(want)}, got ${JSON.stringify(panel[key])}`);
  }
  // Reported, not gated yet: module scripts from an opaque origin (the spike decides).
  console.log(`module scripts: ${panel.module ? 'run' : 'did not run'}; origin ${panel.origin}`);
}
// WebView2 can hold files in the data directory for a moment after the app exits.
try {
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
} catch (e) {
  console.log(`could not remove ${work}: ${e.message}`);
}
if (problems.length) {
  console.error(`panel smoke FAILED:\n  ${problems.join('\n  ')}`);
  process.exit(1);
}
console.log('panel smoke ok');
