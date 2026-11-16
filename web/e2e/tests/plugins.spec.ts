/** Client plugins: installing a theme, trusting third-party code, a plugin that fails,
 * and safe mode. */
import { join } from 'node:path';

import { command, expect, open, test } from './fixtures';
import { writeKkp } from './kkp';

const THEME = `id = "community.ocean"
name = "Ocean"
version = "1.0.0"
kind = ["theme"]
api = "^0.1"
[theme]
variants = [{ id = "night", name = "Night", appearance = "dark", tokens = "night.json" }]
`;

test('a theme package installs and can be chosen at once; bad token values are reported', async ({ page, kk }) => {
  await open(page, kk);
  await kk.pick(
    writeKkp(join(kk.work, 'ocean.kkp'), {
      'plugin.toml': THEME,
      'night.json': JSON.stringify({ accent: '#00b4d8', 'surface.panel': '#03045e', 'text.primary': 'white; color: red' }),
    }),
  );
  await command(page, 'Manage plugins');
  const client = page.getByRole('region', { name: 'On this client' });
  await client.getByRole('button', { name: 'Install from file…' }).click();
  await expect(client.getByRole('listitem').filter({ hasText: 'community.ocean' })).toContainText('Enabled');
  await expect(page.getByText(/text.primary: not a valid color/)).toBeVisible();
  await command(page, 'Theme: Ocean Night');
  const token = (name: string) => page.evaluate((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(), name);
  await expect.poll(() => token('--kk-accent')).toBe('#00b4d8');
  // What the theme leaves out comes from the default theme's dark variant.
  await expect.poll(() => token('--kk-text-primary')).toBe('#e6e8ee');
});

test('third-party UI code runs only after trust, and a failing plugin is reported', async ({ page, kk }) => {
  await open(page, kk);
  await kk.pick(
    writeKkp(join(kk.work, 'bad.kkp'), {
      'plugin.toml': `id = "community.bad"\nname = "Bad"\nversion = "1.0.0"\nkind = ["ui"]\napi = "^0.1"\n[ui]\nentry = "index.js"\n`,
      'index.js': 'export default { activate() { throw new Error("broken on purpose"); } };',
    }),
  );
  await command(page, 'Manage plugins');
  const client = page.getByRole('region', { name: 'On this client' });
  await client.getByRole('button', { name: 'Install from file…' }).click();
  const trust = page.getByRole('dialog', { name: 'Trust Bad?' });
  await expect(trust).toContainText('full access to this app and to every core');
  await trust.getByRole('button', { name: 'Trust and enable' }).click();
  await expect(client.getByRole('listitem').filter({ hasText: 'community.bad' })).toContainText('broken on purpose');
  // The rest of the app still works.
  await command(page, 'Open settings');
  await expect(page.getByRole('navigation', { name: 'Settings pages' })).toContainText('Appearance');
});

test('safe mode loads no UI plugins but keeps themes and the plugin manager', async ({ page, kk }) => {
  await open(page, kk, '?safe');
  await expect(page.getByText(/Safe mode: no UI plugins are loaded/)).toBeVisible();
  await command(page, 'Open settings');
  await expect(page.getByText('No settings pages are installed.')).toBeVisible();
  const panel = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--kk-surface-panel').trim());
  expect(panel).not.toBe('');
  await command(page, 'Manage plugins');
  await expect(page.getByRole('region', { name: 'On this client' })).toContainText('kumikit.ui-chat');
});
