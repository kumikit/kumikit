/**
 * M8: signed application updates in Settings → Updates, including opt-in automatic
 * updates that wait while an agent turn is running and apply once all agents are idle.
 */
import type { Page } from '@playwright/test';

import { command, openAgents, expect, open, test, type Kumikit, modelButton } from './fixtures';

test.use({ updates: true });

async function installRecordedProvider(page: Page, kk: Kumikit, transcript: string) {
  const region = await openAgents(page);
  await kk.pick(kk.provider(transcript, { mode: 'default' }));
  await region.getByRole('button', { name: 'Install from folder…' }).click();
  const grant = page.getByRole('dialog', { name: /Enable Recorded/ });
  await grant.getByRole('button', { name: 'Grant and enable' }).click();
  await expect(region.getByRole('listitem').filter({ hasText: 'test.recorded' }).last()).toContainText('Enabled');
  await command(page, 'Manage plugins');
  await expect(page.getByRole('region', { name: /On this core/ }).getByRole('listitem').filter({ hasText: 'kumikit.provider-acp' })).toContainText('Enabled');
  await page.getByRole('button', { name: 'Close', exact: true }).click();
}

test('Settings → Updates checks signed releases and defers automatic install until agents are idle', async ({
  page,
  kk,
}) => {
  await open(page, kk);

  // Open Settings → Updates.
  await page.getByRole('button', { name: 'Open settings' }).click();
  await page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: 'Updates' }).click();

  await expect(page.locator('[data-app-version]')).toHaveText('v0.1.0-m7.4');
  const autoBox = page.getByLabel(/Automatically download and install updates/);
  await expect(autoBox).not.toBeChecked();

  // Manual check detects the signed v0.1.0-m8 release.
  await page.getByRole('button', { name: 'Check for updates' }).click();
  await expect(page.getByText('Update available: v0.1.0-m8')).toBeVisible();

  // Close settings, start a turn that waits on approval so the core is busy.
  await page.locator('[data-overlay="settings"]').getByRole('button', { name: 'Close', exact: true }).click();
  await installRecordedProvider(page, kk, 'claude-tool-allow');

  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(modelButton(page)).toBeVisible();
  await page.getByRole('button', { name: /Permissions:/ }).click();
  await page.getByRole('menuitemradio', { name: /Standard/ }).click();
  await page.getByLabel('Message').fill('Add a line to notes.txt');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Needs your approval');

  // Turn on opt-in automatic updates while the agent is waiting for approval.
  await page.getByRole('button', { name: 'Open settings' }).click();
  await page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: 'Updates' }).click();
  await autoBox.check();

  // Must download and verify, then WAIT because 1 turn is active — never interrupting it!
  await expect(page.getByText(/Waiting for active agents to finish before installing/)).toBeVisible();
  await expect(page.getByText(/1 running, 0 queued/)).toBeVisible();
  expect(kk.updates.readAppimage()).toBe('old-appimage-v0.1.0-m7.4');

  // Close settings and approve the tool call so the turn finishes and the core becomes idle.
  await page.locator('[data-overlay="settings"]').getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByRole('region', { name: 'Approval' }).getByRole('button', { name: 'Allow', exact: true }).click();

  // Once idle, the automatic updater applies the verified AppImage atomically!
  await expect.poll(() => kk.updates.readAppimage(), { timeout: 15_000 }).toBe(kk.updates.newPayload);
});
