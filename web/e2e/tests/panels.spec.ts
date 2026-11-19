/** Sandboxed panels in the UI kernel (07-ui-plugins, Level 2; ADR 0019): a third-party
 * panel installs without a trust prompt, runs in its frame, reads through the bridge, is
 * refused everything else, and gets theme changes live. The test panel is
 * test-plugins/ui-panel, built by scripts/bundle.mjs. */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

import { ROOT, command, expect, open, test } from './fixtures';
import { writeKkp } from './kkp';

const PANEL = join(ROOT, 'target/bundle/test-ui-plugins/test.ui-panel');

function files(dir: string, root = dir): Record<string, string> {
  const out: Record<string, string> = {};
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) Object.assign(out, files(p, root));
    else out[relative(root, p).split('\\').join('/')] = readFileSync(p, 'utf8');
  }
  return out;
}

test('a sandboxed panel reads through the bridge and is refused the rest', async ({ page, kk }) => {
  // The page's and the panel's console problems go to the test output, for CI failures.
  page.on('console', (m) => {
    if (m.type() === 'error' || m.type() === 'warning') console.log(`PAGE console ${m.type()}: ${m.text()}`);
  });
  page.on('pageerror', (e) => console.log(`PAGE error: ${e.message}`));
  await open(page, kk);
  await kk.pick(writeKkp(join(kk.work, 'panel.kkp'), files(PANEL)));
  await command(page, 'Manage plugins');
  const client = page.getByRole('region', { name: 'On this client' });
  await client.getByRole('button', { name: 'Install from file…' }).click();
  // No trust prompt: the panel cannot reach the app.
  await expect(client.getByRole('listitem').filter({ hasText: 'test.ui-panel' })).toContainText('Enabled');
  await expect(page.getByRole('dialog', { name: /Trust/ })).toHaveCount(0);

  const frame = page.frameLocator('iframe[title="Panel test"]');
  const out = frame.locator('#out[data-ready="1"]');
  await expect(out).toBeVisible({ timeout: 15_000 });
  const results = JSON.parse((await out.textContent())!) as Record<string, unknown>;
  expect(results).toMatchObject({
    slot: 'sidebar.section',
    pluginList: 'ok',
    send: 'permission_denied',
    devices: 'permission_denied',
    install: 'permission_denied',
    shutdown: 'permission_denied',
    network: 'blocked',
    parent: 'blocked',
    storage: 'yes',
  });
  expect(String(results.accent)).not.toBe('');
  // The iframe is sandboxed without same-origin rights.
  await expect(page.locator('iframe[title="Panel test"]')).toHaveAttribute('sandbox', 'allow-scripts');

  // Theme changes reach the panel live.
  const before = results.accent;
  await command(page, 'Theme: Default Dark');
  await expect
    .poll(async () => JSON.parse((await frame.locator('#out').textContent())!).themeChanges ?? 0)
    .toBeGreaterThan(0);
  const after = JSON.parse((await frame.locator('#out').textContent())!) as Record<string, unknown>;
  expect(after.accent).not.toBe('');
  expect(typeof before).toBe('string');
});
