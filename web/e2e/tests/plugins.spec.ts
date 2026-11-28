/** Client plugins and resource themes: importing a `.json` theme file in Appearance,
 * selecting bundled themes, trusting third-party UI code, failing plugins, and safe mode. */
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { command, expect, open, test } from './fixtures';
import { writeKkp } from './kkp';

test('bundled themes preview in Appearance and a custom .json theme imports, applies, and removes', async ({ page, kk }) => {
  await open(page, kk);
  const themePath = join(kk.work, 'ocean-night.json');
  writeFileSync(
    themePath,
    JSON.stringify({
      name: 'Ocean Night',
      appearance: 'dark',
      tokens: {
        accent: '#00b4d8',
        'surface.panel': '#03045e',
        'text.primary': 'white; color: red',
      },
    }),
  );

  await command(page, 'Open settings');
  await page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: 'Appearance' }).click();
  const appearance = page.locator('.kk-appearance');
  await expect(appearance).toBeVisible();
  // Popular bundled themes render with SVG preview thumbnails.
  await expect(appearance.locator('svg.thumb')).not.toHaveCount(0);
  await expect(appearance).toContainText('Tokyo Night');
  await expect(appearance).toContainText('Dracula');
  await expect(appearance).toContainText('Monokai');

  // Import a custom .json theme file from disk.
  const [chooser] = await Promise.all([
    page.waitForEvent('filechooser'),
    appearance.getByRole('button', { name: 'Add theme…' }).click(),
  ]);
  await chooser.setFiles(themePath);

  await expect(page.getByText(/text\.primary: not a valid color/)).toBeVisible();
  await expect(appearance).toContainText('Your themes (1)');
  await expect(appearance).toContainText('Ocean Night');

  const token = (name: string) => page.evaluate((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(), name);
  await expect.poll(() => token('--kk-accent')).toBe('#00b4d8');
  // What the theme leaves out comes from the default dark fallback theme.
  await expect.poll(() => token('--kk-text-primary')).toBe('#e6e8ee');

  // Switch to a bundled popular theme from the palette.
  await command(page, 'Theme: Dracula');
  await expect.poll(() => token('--kk-surface-panel')).toBe('#282a36');

  // Remove the custom theme.
  await appearance.getByRole('button', { name: 'Remove Ocean Night' }).click();
  await expect(appearance).not.toContainText('Ocean Night');
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
