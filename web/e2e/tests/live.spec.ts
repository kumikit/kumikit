/**
 * Live smoke test against real Claude Code (05-providers, Testing providers): needs
 * `claude-agent-acp` on PATH and a Claude login, and spends tokens. Skipped unless
 * KK_LIVE=1. Run: KK_LIVE=1 npx playwright test tests/live.spec.ts --project=chromium
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { command, expect, open, test, modelButton } from './fixtures';

test.skip(!process.env.KK_LIVE, 'live test: set KK_LIVE=1');

test('a real Claude Code turn with an approved file edit', async ({ page, kk }) => {
  test.setTimeout(240_000);
  await open(page, kk);
  await kk.pick(kk.provider('', { mode: 'default', live: true }));
  await command(page, 'Manage plugins');
  await page.getByRole('region', { name: /On this core/ }).getByRole('button', { name: 'Install from folder…' }).click();
  await page.getByRole('button', { name: 'Grant and enable' }).click();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(modelButton(page)).toBeVisible();
  await page.getByRole('button', { name: /Permissions:/ }).click();
  await page.getByRole('menuitemradio', { name: /Standard/ }).click();
  await page.getByLabel('Message').fill('Create a file named hello.txt in this folder containing exactly the word hi. Do nothing else.');
  await page.getByLabel('Message').press('Enter');
  const approval = page.getByRole('region', { name: 'Approval' }).filter({ hasText: 'Needs your approval' }).first();
  await expect(approval).toBeVisible({ timeout: 180_000 });
  await approval.getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText(' out', { timeout: 180_000 });
  expect(readFileSync(join(kk.project, 'hello.txt'), 'utf8').trim()).toBe('hi');
});
