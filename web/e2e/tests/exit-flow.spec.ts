/**
 * M2's exit criteria in the browser build: install an ACP provider, start a thread,
 * approve a tool call, switch themes; reopening the page resumes the running turn.
 * The agent replays a recorded Claude Code transcript.
 */
import { command, expect, open, test } from './fixtures';

test('install a provider, approve a tool call across a reopen, switch themes', async ({ page, kk }) => {
  await open(page, kk);

  // provider-acp is preinstalled on the core from the bundle.
  await command(page, 'Manage plugins');
  const core = page.getByRole('region', { name: /On this core/ });
  await expect(core.getByRole('listitem').filter({ hasText: 'kumikit.provider-acp' })).toContainText('Enabled');

  // Install a declarative provider from its folder; its permission is shown in words.
  await kk.pick(kk.provider('claude-tool-allow', { mode: 'default' }));
  await core.getByRole('button', { name: 'Install from folder…' }).click();
  const grant = page.getByRole('dialog', { name: /Enable Recorded/ });
  await expect(grant).toContainText('can start the `fake-acp-agent` program, which runs with your full user access, outside the sandbox');
  await expect(grant).toContainText('not signed');
  await grant.getByRole('button', { name: 'Grant and enable' }).click();
  await expect(core.getByRole('listitem').filter({ hasText: 'test.recorded1' })).toContainText('Enabled');
  await page.getByRole('button', { name: 'Close', exact: true }).click();

  // Open a project and start a thread with it.
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true })).toBeVisible();
  await page.getByLabel('Agent').selectOption({ label: 'Recorded agent 1' });
  await page.getByRole('button', { name: /Permissions:/ }).click();
  await page.getByRole('menuitemradio', { name: /Standard/ }).click();

  await page.getByLabel('Message').fill('Add a line to notes.txt');
  await page.getByLabel('Message').press('Enter');
  const approval = page.getByRole('region', { name: 'Approval' });
  await expect(approval).toContainText('Needs your approval');
  await expect(page.getByRole('navigation', { name: 'Workspaces', exact: true })).toContainText('waiting for approval');

  // "Close and reopen the window": the turn keeps running in the core.
  await page.reload();
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Needs your approval');
  await expect(page.getByRole('log', { name: 'Transcript' }).getByText('Add a line to notes.txt')).toHaveCount(1);

  await page.getByRole('region', { name: 'Approval' }).getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Allowed');
  await expect(page.getByRole('button', { name: /changed files?/ }).first()).toBeVisible();
  await expect(page.getByText('Idle', { exact: true })).toBeVisible();

  // Switch themes from the palette; the tokens change at once.
  const panel = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--kk-surface-panel').trim());
  await command(page, 'Theme: Kumikit Dark');
  await expect.poll(panel).toBe('#15171c');
  await command(page, 'Theme: Kumikit Light');
  await expect.poll(panel).toBe('#ffffff');
});
