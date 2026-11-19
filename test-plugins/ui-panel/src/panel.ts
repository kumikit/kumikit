// Probes the bridge from inside a sandboxed panel and writes what happened into #out,
// as JSON, for the tests to read.
import { connectPanel, PanelError } from '@kumikit/ui-sdk/panel';

const out = document.getElementById('out')!;
const results: Record<string, unknown> = {};
const show = () => {
  out.textContent = JSON.stringify(results);
  out.dataset.ready = '1';
};
const code = (e: unknown) => (e instanceof PanelError ? e.code : String(e));

const ctx = await connectPanel();
results.slot = ctx.slot;
results.accent = getComputedStyle(document.documentElement).getPropertyValue('--kk-accent').trim();
try {
  const r = (await ctx.connection.call('plugin.list', {})) as { plugins: unknown[] };
  results.pluginList = Array.isArray(r.plugins) ? 'ok' : 'odd';
} catch (e) {
  results.pluginList = code(e);
}
for (const [name, method, params] of [
  ['send', 'thread.send', { thread: 'x', input: { text: 'hi' } }],
  ['devices', 'auth.devices.list', {}],
  ['install', 'plugin.install', { registry: { id: 'x.y' } }],
  ['shutdown', 'core.shutdown', {}],
] as const) {
  try {
    await ctx.connection.call(method, params);
    results[name] = 'ALLOWED';
  } catch (e) {
    results[name] = code(e);
  }
}
try {
  await fetch('https://kumikit-smoke.invalid/');
  results.network = 'REACHED';
} catch {
  results.network = 'blocked';
}
results.parent = (() => {
  try {
    void parent.document.title;
    return 'READABLE';
  } catch {
    return 'blocked';
  }
})();
ctx.theme.onChange(() => {
  results.accent = getComputedStyle(document.documentElement).getPropertyValue('--kk-accent').trim();
  results.themeChanges = Number(results.themeChanges ?? 0) + 1;
  show();
});
await ctx.storage.set('seen', 'yes');
results.storage = await ctx.storage.get('seen');
show();
