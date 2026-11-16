// Assembles the desktop app's resources in target/bundle (docs/plans M2, step 10):
//   ui/            the UI kernel (desktop build) and bundled.json
//   ui-browser-test/ the browser-test build, for Playwright
//   ui-plugins/<id>/ the bundled UI plugins and themes, unpacked (read-only)
//   core-plugins/  provider-acp as a .kkp, seeded on the core with --bundled-plugins
// Usage: node scripts/bundle.mjs   (from the repository root)
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const OUT = 'target/bundle';
const UI_PLUGINS = ['plugins/ui-chat', 'plugins/ui-settings', 'plugins/theme-default'];
const FALLBACK_THEME = 'kumikit.theme-default';
const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: 'inherit', ...opts });
const tomlString = (text, key) => new RegExp(`^${key}\\s*=\\s*"([^"]+)"`, 'm').exec(text)?.[1];

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

// The UI kernel, both builds.
run('npm', ['run', 'build', '-w', '@kumikit/kernel']);
const bundled = JSON.stringify({ fallbackTheme: FALLBACK_THEME }, null, 2) + '\n';
for (const [from, to] of [['web/kernel/dist/desktop', 'ui'], ['web/kernel/dist/browser-test', 'ui-browser-test']]) {
  cpSync(from, join(OUT, to), { recursive: true });
  writeFileSync(join(OUT, to, 'bundled.json'), bundled);
}

// UI plugins and themes: the manifest and the files it names.
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
  for (const m of manifest.matchAll(/tokens\s*=\s*"([^"]+)"/g)) cpSync(join(dir, m[1]), join(target, m[1]));
  for (const m of manifest.matchAll(/"(fonts\/[^"]+\.woff2)"/g)) cpSync(join(dir, m[1]), join(target, m[1]));
}

// provider-acp: a release component, packed like `kumikit plugin pack`.
run('cargo', ['build', '-q', '--release', '-p', 'kumikit-provider-acp', '--target', 'wasm32-wasip2']);
run('cargo', ['build', '-q', '-p', 'kumikit-core']);
const target = process.env.CARGO_TARGET_DIR ?? 'target';
const stage = join(OUT, 'stage-provider-acp');
mkdirSync(stage, { recursive: true });
for (const f of ['plugin.toml', 'acp.schema.json']) cpSync(join('plugins/provider-acp', f), join(stage, f));
cpSync(join(target, 'wasm32-wasip2/release/kumikit_provider_acp.wasm'), join(stage, 'plugin.wasm'));
const version = tomlString(readFileSync('plugins/provider-acp/plugin.toml', 'utf8'), 'version');
mkdirSync(join(OUT, 'core-plugins'), { recursive: true });
const kumikit = join(target, 'debug', process.platform === 'win32' ? 'kumikit.exe' : 'kumikit');
run(kumikit, ['plugin', 'pack', stage, '-o', join(OUT, 'core-plugins', `kumikit.provider-acp-${version}.kkp`)]);
rmSync(stage, { recursive: true, force: true });
console.log(`bundle ready in ${OUT}`);
