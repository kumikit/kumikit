/**
 * Plugins from a registry in the browser build (10-distribution; 06-client, Plugin
 * manager): a fixture registry on loopback, publisher "acme" with a key made for the test
 * (test-plugins/registry-fixture), and the relay's core and client registry pointed at it.
 */
import { cpSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import AxeBuilder from '@axe-core/playwright';

import { ROOT, command, openAgents, expect, open, test } from './fixtures';

test.use({ registry: true });

/** The test panel, republished by acme as `acme.panel`. */
function acmePanel(work: string): string {
  const dir = join(work, 'acme-panel');
  cpSync(join(ROOT, 'target/bundle/test-ui-plugins/test.ui-panel'), dir, { recursive: true });
  const manifest = join(dir, 'plugin.toml');
  writeFileSync(manifest, readFileSync(manifest, 'utf8').replace('id = "test.ui-panel"', 'id = "acme.panel"').replace('name = "Panel test"', 'name = "Acme Panel"'));
  return dir;
}

test('the starter pack contacts nothing until asked and installs only what is chosen', async ({ page, kk }) => {
  const r = kk.registry;
  await r.publish(await r.probe('0.1.0'));
  await r.publish(acmePanel(kk.work));
  await r.starter('acme.probe', 'acme.panel');
  await r.build();
  const before = await r.hits();

  await open(page, kk);
  const welcome = page.getByRole('region', { name: 'Welcome to Kumikit' });
  await expect(welcome).toContainText(new URL(r.url).host);
  expect(await r.hits(), 'opening the app contacts no registry').toBe(before);

  await welcome.getByRole('button', { name: 'Browse the starter pack' }).click();
  const pack = page.getByRole('region', { name: 'Starter pack' });
  const probe = pack.getByRole('listitem').filter({ hasText: 'acme.probe' });
  await expect(probe).toContainText('Published by ACME');
  await expect(probe).toContainText('can start the `fake-acp-agent` program');
  await expect(pack.getByRole('listitem').filter({ hasText: 'acme.panel' })).toContainText('runs in a sandboxed panel');
  expect(await r.hits()).toBeGreaterThan(before);
  const axe = await new AxeBuilder({ page: page as never }).include('main').withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  expect(axe.violations.map((v) => v.id)).toEqual([]);

  // Install one; the other stays out.
  await probe.getByRole('button', { name: /Install on/ }).click();
  const grant = page.getByRole('dialog', { name: /Enable Acme Probe/ });
  await expect(grant).toContainText('the publisher’s key in the registry');
  await expect(grant).toContainText('can start the `fake-acp-agent` program');
  await grant.getByRole('button', { name: 'Grant and enable' }).click();
  // The probe is an agent, so it is listed in Settings → Agents.
  const agents = await openAgents(page);
  await expect(agents.getByRole('listitem').filter({ hasText: 'acme.probe' })).toContainText('Enabled');
  await command(page, 'Manage plugins');
  await expect(page.getByRole('region', { name: 'On this core' })).not.toContainText('acme.probe');
  await expect(page.getByRole('region', { name: 'On this client' })).not.toContainText('acme.panel');
  // Answered: the welcome does not come back.
  await page.reload();
  await expect(page.getByRole('region', { name: 'Welcome to Kumikit' })).toHaveCount(0);
});

test('a panel from the registry installs without a trust prompt', async ({ page, kk }) => {
  const r = kk.registry;
  await r.publish(acmePanel(kk.work));
  await r.build();
  await open(page, kk);
  await command(page, 'Manage plugins');
  await page.getByLabel('Search the registries').fill('panel');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.getByRole('list', { name: 'Registry results' }).getByRole('listitem').filter({ hasText: 'acme.panel' }).getByRole('button', { name: 'Install in this app' }).click();
  await expect(page.getByRole('region', { name: 'On this client' }).getByRole('listitem').filter({ hasText: 'acme.panel' })).toContainText('Signed with key');
  await expect(page.getByRole('dialog', { name: /Trust/ })).toHaveCount(0);
  const out = page.frameLocator('iframe[title="Panel test"]').locator('#out[data-ready="1"]');
  await expect(out).toBeVisible({ timeout: 15_000 });
  expect(JSON.parse((await out.textContent())!)).toMatchObject({ send: 'permission_denied', pluginList: 'ok' });
});

test('updates wait to be asked for, and wider ones for the new permissions', async ({ page, kk }) => {
  const r = kk.registry;
  await r.publish(await r.probe('0.1.0'));
  await r.build();
  await open(page, kk);
  const agents = await openAgents(page);
  // Settings → Agents finds agents in the registries.
  await page.getByLabel('Search agents').fill('probe');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.getByRole('list', { name: 'Available agents' }).getByRole('button', { name: 'Install…' }).click();
  await page.getByRole('dialog', { name: /Enable Acme Probe/ }).getByRole('button', { name: 'Grant and enable' }).click();
  const installed = agents.getByRole('listitem').filter({ hasText: 'acme.probe' });
  await expect(installed).toContainText('v0.1.0');

  await r.publish(await r.probe('0.2.0', 'secrets = ["acme-token"]'));
  await r.build();
  await agents.getByRole('button', { name: 'Check for updates' }).click();
  await expect(installed).toContainText('Version 0.2.0 is available; it asks for more permissions');
  const offer = installed;
  await offer.getByRole('button', { name: 'Review and update…' }).click();
  let dialog = page.getByRole('dialog', { name: 'Update acme.probe to 0.2.0?' });
  await expect(dialog).toContainText('can read the secret `acme-token`');
  await dialog.getByRole('button', { name: 'Not now' }).click();
  await expect(installed).toContainText('v0.1.0');
  await offer.getByRole('button', { name: 'Review and update…' }).click();
  dialog = page.getByRole('dialog', { name: 'Update acme.probe to 0.2.0?' });
  await dialog.getByRole('button', { name: 'Grant and update' }).click();
  await expect(installed).toContainText('v0.2.0');
  await expect(installed).not.toContainText('is available');
  await expect(installed).toContainText('can read the secret `acme-token`');

  // A search in the plugin manager leaves the agent out.
  await command(page, 'Manage plugins');
  await page.getByLabel('Search the registries').fill('probe');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Nothing found.' })).toBeVisible();

  // The lockfile says so.
  await page.getByRole('region', { name: 'Registries and lockfiles' }).getByRole('button', { name: 'Show' }).click();
  await page.getByRole('region', { name: 'Registries and lockfiles' }).getByRole('button', { name: 'Export lockfile' }).first().click();
  const lock = page.getByRole('dialog', { name: /Lockfile of/ }).getByLabel('Lockfile');
  await expect(lock).toHaveValue(/id = "acme.probe"\nversion = "0.2.0"/);
});

test('a lying registry installs nothing', async ({ page, kk }) => {
  const r = kk.registry;
  await r.publish(await r.probe('0.1.0'), 'other-key');
  await r.build();
  await open(page, kk);
  const agents = await openAgents(page);
  await page.getByLabel('Search agents').fill('probe');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.getByRole('list', { name: 'Available agents' }).getByRole('button', { name: 'Install…' }).click();
  await expect(page.getByText(/not by the publisher's key/)).toBeVisible();
  await expect(agents).not.toContainText('acme.probe');
});
