/**
 * M4's exit criteria in the browser build (12-roadmap): a plugin published to the registry
 * by a third party installs on a fresh machine, with its permissions shown and enforced.
 * The publisher "acme" and its key are made for this test (test-plugins/registry-fixture);
 * the plugin is the probe test component, which reports host errors unchanged.
 */
import { command, expect, open, test } from './fixtures';

test.use({ registry: true });

test('a third-party plugin from the registry installs with its permissions shown and enforced', async ({ page, kk }) => {
  const r = kk.registry;
  await r.publish(await r.probe('0.1.0'));
  await r.starter('acme.probe');
  await r.build();

  // A fresh client: the welcome asks before anything is fetched.
  await open(page, kk);
  const before = await r.hits();
  await page.getByRole('region', { name: 'Welcome to Kumikit' }).getByRole('button', { name: 'Browse the starter pack' }).click();
  const probe = page.getByRole('region', { name: 'Starter pack' }).getByRole('listitem').filter({ hasText: 'acme.probe' });
  await expect(probe).toContainText('Published by ACME');
  expect(await r.hits()).toBeGreaterThan(before);

  // Shown: who made it, how it is signed, and what it may do.
  await probe.getByRole('button', { name: /Install on/ }).click();
  const grant = page.getByRole('dialog', { name: /Enable Acme Probe/ });
  await expect(grant).toContainText('from acme');
  await expect(grant).toContainText('https://example.com/acme/probe');
  await expect(grant).toContainText('the publisher’s key in the registry');
  await expect(grant).toContainText('can start the `fake-acp-agent` program, which runs with your full user access, outside the sandbox');
  await expect(grant).toContainText('It starts programs outside the sandbox');
  await grant.getByRole('button', { name: 'Grant and enable' }).click();

  // Enforced: a thread with it; what was granted works, anything else is refused.
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true })).toBeVisible();
  await command(page, 'New thread');
  const dialog = page.getByRole('dialog', { name: 'New thread' });
  await dialog.getByLabel('Agent').selectOption({ label: 'Acme Probe' });
  await dialog.getByRole('button', { name: 'Start' }).click();

  // Granted: the probe starts `fake-acp-agent`, which answers with its working directory.
  await page.getByLabel('Message').fill('report fake-acp-agent');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByText(kk.project, { exact: false }).first()).toBeVisible();
  await expect(page.getByText(/permission_denied/)).toHaveCount(0);

  await page.getByLabel('Message').fill('spawn sh');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByText(/permission_denied: `sh` is not in permissions.spawn/)).toBeVisible();

  await page.getByLabel('Message').fill('secret anthropic-api-key');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByText(/permission_denied: secret `anthropic-api-key` is not in permissions.secrets/)).toBeVisible();
});
