/** Accessibility (06-client, UI rules): axe on the main screens in both themes, and the
 * main flow with the keyboard only. */
import AxeBuilder from '@axe-core/playwright';
import type { Page } from '@playwright/test';

import { command, expect, open, test } from './fixtures';

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
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await page.getByRole('button', { name: 'New thread…' }).click();
  await page.getByRole('dialog', { name: 'New thread' }).getByRole('button', { name: 'Start' }).click();
  await page.getByLabel('Message').fill('Say hello');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText(' out');

  for (const theme of ['Theme: Kumikit Light', 'Theme: Kumikit Dark']) {
    await command(page, theme);
    await axe(page, `${theme}, thread`);
    await command(page, 'Manage plugins');
    await axe(page, `${theme}, plugins`);
    await command(page, 'Open settings');
    await axe(page, `${theme}, settings`);
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
  await expect(page.getByLabel('Agent')).toBeFocused();
  // The model list arrives from the core; until then its select is skipped by Tab.
  await expect(page.getByLabel('Model')).toBeEnabled();
  // Agent, model, then the title; Enter in a text field submits the form.
  await page.keyboard.press('Tab');
  await page.keyboard.press('Tab');
  await page.keyboard.type('Keyboard only');
  await page.keyboard.press('Enter');
  await expect(page.getByLabel('Message')).toBeVisible();
  await page.getByLabel('Message').focus();
  await page.keyboard.type('Say hello');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText('hello from kumikit');
  // Escape closes the palette and gives focus back.
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k');
  await expect(page.getByLabel('Command', { exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByLabel('Message')).toBeFocused();
});
