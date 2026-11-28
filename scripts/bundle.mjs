// Assembles the desktop app's resources in target/bundle (docs/plans M2, step 10):
//   ui/            the UI kernel (desktop build), bundled.json, and themes/
//   ui-browser/    the browser client, for `kumikit serve --listen … --web target/bundle/ui-browser`:
//                  the browser build, bundled.json, themes/, and the bundled UI plugins in ui-plugins/
//   ui-browser-test/ the browser-test build, for Playwright
//   ui-plugins/<id>/ the bundled UI plugins, unpacked (read-only)
//   core-plugins/  provider-acp and threads as .kkp packages, seeded with --bundled-plugins
//   test-ui-plugins/<id>/ UI plugins for the tests only, unpacked (never shipped)
// Usage: node scripts/bundle.mjs   (from the repository root)
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const OUT = 'target/bundle';
const UI_PLUGINS = ['plugins/ui-chat', 'plugins/ui-settings', 'plugins/ui-usage'];
const FALLBACK_LIGHT = 'kumikit-light';
const FALLBACK_DARK = 'kumikit-dark';
// npm and npx are .cmd files on Windows, which only a shell can start.
const run = (cmd, args, opts = {}) =>
  execFileSync(cmd, args, { stdio: 'inherit', shell: process.platform === 'win32' && ['npm', 'npx'].includes(cmd), ...opts });
const tomlString = (text, key) => new RegExp(`^${key}\\s*=\\s*"([^"]+)"`, 'm').exec(text)?.[1];

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

// Load bundled themes (themes/*.json, ADR 0037), defaults first then alphabetical.
const themeFiles = readdirSync('themes')
  .filter((f) => f.endsWith('.json') && f !== 'index.json')
  .sort((a, b) => {
    const rank = (f) => (f === `${FALLBACK_LIGHT}.json` ? 0 : f === `${FALLBACK_DARK}.json` ? 1 : 2);
    return rank(a) - rank(b) || a.localeCompare(b);
  });
const themesIndex = themeFiles.map((file) => {
  const doc = JSON.parse(readFileSync(join('themes', file), 'utf8'));
  return { id: doc.id, name: doc.name, appearance: doc.appearance, tokens: doc.tokens };
});
const themesIndexJson = JSON.stringify(themesIndex) + '\n';

// The UI kernel, all three builds.
run('npm', ['run', 'build', '-w', '@kumikit/kernel']);
const bundled = JSON.stringify({ fallbackLight: FALLBACK_LIGHT, fallbackDark: FALLBACK_DARK }, null, 2) + '\n';
for (const [from, to] of [['web/kernel/dist/desktop', 'ui'], ['web/kernel/dist/browser', 'ui-browser'], ['web/kernel/dist/browser-test', 'ui-browser-test']]) {
  cpSync(from, join(OUT, to), { recursive: true });
  writeFileSync(join(OUT, to, 'bundled.json'), bundled);
  mkdirSync(join(OUT, to, 'themes'), { recursive: true });
  writeFileSync(join(OUT, to, 'themes', 'index.json'), themesIndexJson);
}

// UI plugins: the manifest and the files it names.
for (const dir of UI_PLUGINS) {
  const manifest = readFileSync(join(dir, 'plugin.toml'), 'utf8');
  const id = tomlString(manifest, 'id');
  const target = join(OUT, 'ui-plugins', id);
  if (existsSync(join(dir, 'package.json')) && existsSync(join(dir, 'vite.config.ts'))) {
    run('npx', ['vite', 'build', '--logLevel', 'warn'], { cwd: dir });
  }
  mkdirSync(target, { recursive: true });
  cpSync(join(dir, 'plugin.toml'), join(target, 'plugin.toml'));
  if (existsSync(join(dir, 'dist'))) cpSync(join(dir, 'dist'), join(target, 'dist'), { recursive: true });
}

// Test-only UI plugins (never shipped): built here, staged in test-ui-plugins/ for the
// browser and desktop suites.
for (const dir of ['test-plugins/ui-panel']) {
  const manifest = readFileSync(join(dir, 'plugin.toml'), 'utf8');
  const id = tomlString(manifest, 'id');
  const target = join(OUT, 'test-ui-plugins', id);
  run('npx', ['vite', 'build', '--logLevel', 'warn'], { cwd: dir });
  mkdirSync(target, { recursive: true });
  for (const f of ['plugin.toml', 'index.html', 'panel.css']) cpSync(join(dir, f), join(target, f));
  cpSync(join(dir, 'dist'), join(target, 'dist'), { recursive: true });
}

// The browser client serves the same UI plugins itself.
cpSync(join(OUT, 'ui-plugins'), join(OUT, 'ui-browser', 'ui-plugins'), { recursive: true });

// Bundled core plugins: release components, packed like `kumikit plugin pack`.
// CI can pass KUMIKIT_BIN, KUMIKIT_PROVIDER_ACP_WASM, and KUMIKIT_THREADS_WASM to reuse
// already-built artifacts.
const target = process.env.CARGO_TARGET_DIR ?? 'target';
let acpWasm = process.env.KUMIKIT_PROVIDER_ACP_WASM;
let threadsWasm = process.env.KUMIKIT_THREADS_WASM;
const toBuild = [];
if (!acpWasm) {
  toBuild.push('-p', 'kumikit-provider-acp');
  acpWasm = join(target, 'wasm32-wasip2/release/kumikit_provider_acp.wasm');
}
if (!threadsWasm) {
  toBuild.push('-p', 'kumikit-threads');
  threadsWasm = join(target, 'wasm32-wasip2/release/kumikit_threads.wasm');
}
if (toBuild.length > 0) {
  run('cargo', ['build', '-q', '--release', ...toBuild, '--target', 'wasm32-wasip2']);
}
let kumikit = process.env.KUMIKIT_BIN;
if (!kumikit) {
  run('cargo', ['build', '-q', '-p', 'kumikit-core']);
  kumikit = join(target, 'debug', process.platform === 'win32' ? 'kumikit.exe' : 'kumikit');
}
mkdirSync(join(OUT, 'core-plugins'), { recursive: true });

function packCorePlugin(srcDir, wasmPath, extraFiles) {
  const manifest = readFileSync(join(srcDir, 'plugin.toml'), 'utf8');
  const id = tomlString(manifest, 'id');
  const version = tomlString(manifest, 'version');
  const stage = join(OUT, `stage-${id}`);
  mkdirSync(stage, { recursive: true });
  cpSync(join(srcDir, 'plugin.toml'), join(stage, 'plugin.toml'));
  for (const f of extraFiles) cpSync(join(srcDir, f), join(stage, f), { recursive: true });
  cpSync(wasmPath, join(stage, 'plugin.wasm'));
  run(kumikit, ['plugin', 'pack', stage, '-o', join(OUT, 'core-plugins', `${id}-${version}.kkp`)]);
  rmSync(stage, { recursive: true, force: true });
}

packCorePlugin('plugins/provider-acp', acpWasm, ['acp.schema.json']);
packCorePlugin('plugins/threads', threadsWasm, ['tools']);
console.log(`bundle ready in ${OUT}`);
