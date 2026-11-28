/**
 * Settings → Agents (06-client, Agents): an agent installed from a folder shows on the
 * Agents page with its settings in place, stays out of the plugin manager and the
 * Settings core pages, and is removed there.
 */
import { cpSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { command, expect, open, openAgents, test } from './fixtures';

test.use({ registry: true });

test('an agent is installed, configured and removed in Settings → Agents', async ({ page, kk }) => {
  // The probe test provider, with a settings schema.
  const dir = join(kk.work, 'probe-settings');
  cpSync(await kk.registry.probe('0.1.0'), dir, { recursive: true });
  const manifest = join(dir, 'plugin.toml');
  writeFileSync(manifest, readFileSync(manifest, 'utf8').replace('[contributes]\n', '[contributes]\nsettings = "settings.schema.json"\n'));
  writeFileSync(join(dir, 'settings.schema.json'), JSON.stringify({ type: 'object', properties: { endpoint: { type: 'string', title: 'Endpoint' } } }));

  await open(page, kk);
  const installed = await openAgents(page);
  await expect(installed).toContainText('No agent is installed');
  await kk.pick(dir);
  await installed.getByRole('button', { name: 'Install from folder…' }).click();
  await page.getByRole('dialog', { name: /Enable Acme Probe/ }).getByRole('button', { name: 'Grant and enable' }).click();
  const card = installed.getByRole('listitem').filter({ hasText: 'acme.probe' });
  await expect(card).toContainText('Enabled');

  // Its settings are in its card, saved on the core.
  await card.getByText('Settings', { exact: true }).click();
  const endpoint = card.getByLabel('Endpoint');
  await endpoint.fill('https://example.test');
  await endpoint.press('Tab');
  await page.reload();
  await openAgents(page);
  await card.getByText('Settings', { exact: true }).click();
  await expect(card.getByLabel('Endpoint')).toHaveValue('https://example.test');

  // The agent is not a settings page of its own, nor in the plugin manager.
  await expect(page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: 'acme.probe' })).toHaveCount(0);
  await command(page, 'Manage plugins');
  const core = page.getByRole('region', { name: 'On this core' });
  await expect(core).toContainText('kumikit.provider-acp');
  await expect(core).not.toContainText('acme.probe');
  await expect(core).toContainText('One agent is installed here too.');
  await core.getByRole('button', { name: 'Settings → Agents' }).click();

  // Disabled, it stays an agent; then it is removed.
  await card.getByRole('button', { name: 'Disable' }).click();
  await expect(card).toContainText('Disabled');
  await card.getByRole('button', { name: 'Remove…' }).click();
  await page.getByRole('dialog', { name: 'Remove Acme Probe?' }).getByRole('button', { name: 'Remove and delete data' }).click();
  await expect(installed).toContainText('No agent is installed');
});
