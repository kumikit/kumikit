/** Accessibility (06-client, UI rules): axe on the main screens in both themes, and the
 * main flow with the keyboard only. */
import AxeBuilder from '@axe-core/playwright';
import type { Page } from '@playwright/test';

import { command, expect, mod, open, test, modelButton } from './fixtures';

async function axe(page: Page, where: string) {
  // AxeBuilder's types come from a newer playwright-core than the test runner's.
  const r = await new AxeBuilder({ page: page as never }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  const problems = r.violations.map((v) => `${where}: ${v.id} (${v.impact}): ${v.nodes.map((n) => `${n.target.join(' ')} ${n.html.slice(0, 80)} ${n.failureSummary?.replace(/\s+/g, ' ').slice(0, 200)}`).slice(0, 3).join(', ')}`);
  expect(problems).toEqual([]);
}

test('the main screens have no axe violations, light and dark', async ({ page, kk }) => {
  // With reduced motion every transition is instant, so axe never samples a color
  // halfway through a theme change; this also checks that path.
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await open(page, kk);
  await kk.pick(kk.provider('claude-text'));
  await command(page, 'Manage plugins');
  await page.getByRole('region', { name: /On this core/ }).getByRole('button', { name: 'Install from folder…' }).click();
  await page.getByRole('button', { name: 'Grant and enable' }).click();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(modelButton(page)).toBeVisible();
  await page.getByLabel('Message').fill('Say hello');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText(' out');

  for (const theme of ['Theme: Kumikit Light', 'Theme: Kumikit Dark']) {
    await command(page, theme);
    await axe(page, `${theme}, thread`);
    // The model picker, with the recorded agent's Effort setting (ADR 0033).
    await modelButton(page).click();
    const picker = page.getByRole('dialog', { name: 'Choose a model' });
    await expect(picker.getByRole('radiogroup', { name: 'Effort' })).toBeVisible();
    await axe(page, `${theme}, model picker`);
    await page.keyboard.press('Escape');
    await expect(picker).toHaveCount(0);
    await command(page, 'Manage plugins');
    await axe(page, `${theme}, plugins`);
    await command(page, 'Open settings');
    await axe(page, `${theme}, settings`);
    await page.getByRole('button', { name: 'Close', exact: true }).click();
    await page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true }).click();
  }
});

test('the main flow works with the keyboard alone', async ({ page, kk }) => {
  await open(page, kk);
  await kk.pick(kk.provider('claude-text'));
  await command(page, 'Manage plugins');
  // Tab to the install button and press it.
  const install = page.getByRole('region', { name: /On this core/ }).getByRole('button', { name: 'Install from folder…' });
  await install.focus();
  await page.keyboard.press('Enter');
  // The grant dialog takes focus; its default button grants.
  await expect(page.getByRole('button', { name: 'Grant and enable' })).toBeFocused();
  await page.keyboard.press('Enter');
  await kk.pick(kk.project);
  await command(page, 'Open a project folder');
  await expect(page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true })).toBeVisible();
  await command(page, 'New thread');
  await expect(page.getByLabel('Message')).toBeFocused();
  // The model list arrives from the core; wait until it is enabled before sending.
  await expect(modelButton(page)).toBeEnabled();
  await page.keyboard.type('Say hello');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText('hello from kumikit');
  // Escape closes the palette and gives focus back.
  await page.getByLabel('Message').focus();
  await mod(page, 'k');
  await expect(page.getByLabel('Command', { exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByLabel('Message')).toBeFocused();
});
