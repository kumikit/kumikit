/** Usage statistics and plan limits (ADR 0035): the thread header's meter shows the
 * provider's fullest window, opens the Agent usage dialog on that provider, Refresh reads
 * the limits again, and both pass axe in light and dark. */
import AxeBuilder from '@axe-core/playwright';
import type { Page } from '@playwright/test';

import { chooseAgent, command, expect, open, test } from './fixtures';

test.use({ registry: true });

async function axe(page: Page, where: string) {
  // AxeBuilder's types come from a newer playwright-core than the test runner's.
  const r = await new AxeBuilder({ page: page as never }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  const problems = r.violations.map((v) => `${where}: ${v.id} (${v.impact}): ${v.nodes.map((n) => `${n.target.join(' ')} ${n.html.slice(0, 80)} ${n.failureSummary?.replace(/\s+/g, ' ').slice(0, 200)}`).slice(0, 3).join(', ')}`);
  expect(problems).toEqual([]);
}

test('the usage meter opens Agent usage on the thread’s provider', async ({ page, kk }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await open(page, kk);
  // The probe test provider reports limits when asked to, and reads them on demand.
  await kk.pick(await kk.registry.probe('0.1.0'));
  await command(page, 'Manage plugins');
  await page.getByRole('region', { name: /On this core/ }).getByRole('button', { name: 'Install from folder…' }).click();
  await page.getByRole('button', { name: 'Grant and enable' }).click();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await chooseAgent(page, 'Acme Probe');
  await page.getByLabel('Message').fill('limits 86');
  await page.getByLabel('Message').press('Enter');

  const meter = page.getByRole('button', { name: /^Acme Probe: 5 hours limit 86% used/ });
  await expect(meter).toBeVisible();
  await expect(meter).toHaveText(/5h\s*86%/);

  for (const theme of ['Theme: Kumikit Light', 'Theme: Kumikit Dark']) {
    await command(page, theme);
    await axe(page, `${theme}, thread header`);
    await meter.click();
    const dialog = page.getByRole('dialog', { name: 'Agent usage' });
    await expect(dialog.getByRole('tab', { name: 'Acme Probe' })).toHaveAttribute('aria-selected', 'true');
    await expect(dialog.getByRole('meter', { name: '5 hours' })).toHaveAttribute('aria-valuenow', '86');
    await expect(dialog.getByRole('heading', { name: 'Past 30 days' })).toBeVisible();
    await axe(page, `${theme}, Agent usage`);
    await dialog.getByRole('tab', { name: 'Overview' }).click();
    await expect(dialog.getByRole('table', { name: 'Providers' })).toContainText('Acme Probe');
    await axe(page, `${theme}, Agent usage overview`);
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(meter).toBeFocused();
  }

  // Refresh runs the provider's reader (its first read reports 10 %).
  await meter.click();
  const dialog = page.getByRole('dialog', { name: 'Agent usage' });
  await dialog.getByRole('button', { name: 'Refresh Acme Probe plan limits' }).click();
  await expect(dialog.getByRole('meter', { name: '5 hours' })).toHaveAttribute('aria-valuenow', '10');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: /^Acme Probe: 5 hours limit 10% used/ })).toBeVisible();
});
