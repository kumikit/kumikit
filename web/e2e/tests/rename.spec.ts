/**
 * Thread rename from the sidebar (06-client, Sidebar): each thread row has a `⋯` icon
 * button beside it, and both it and the right-click menu offer Rename. A renamed thread
 * keeps its new title; Escape leaves it as it was.
 */
import { expect, open, test } from './fixtures';

test('a thread is renamed from its row menu, and Escape cancels', async ({ page, kk }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await open(page, kk);

  // Install a recorded agent provider, so a chat can run a turn.
  await kk.pick(kk.provider('claude-text'));
  await page.locator('footer').getByRole('button', { name: 'Manage plugins' }).click();
  await page.getByRole('region', { name: /On this core/ }).getByRole('button', { name: 'Install from folder…' }).click();
  await page.getByRole('button', { name: 'Grant and enable' }).click();
  await page.locator('[data-overlay-scrim]').click({ position: { x: 8, y: 8 } });
  await expect(page.locator('[data-overlay="plugins"]')).toHaveCount(0);
  // Start a chat and let its first turn finish so the thread is idle (rename is refused mid-turn).
  await page.locator('header.topbar').getByRole('button', { name: 'New chat' }).click();
  await page.getByLabel('Message').fill('Rename me please');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText('hello from kumikit');

  const sidebar = page.getByRole('navigation', { name: 'Workspaces', exact: true });
  const row = sidebar.locator('li.row', { hasText: 'Rename me please' });
  await expect(row.locator('.kk-status')).toHaveClass(/idle/);

  // The icon button opens the same menu the right-click does.
  await row.getByRole('button', { name: 'Thread actions' }).click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();

  const dialog = page.getByRole('dialog', { name: 'Rename thread' });
  const field = dialog.getByRole('textbox', { name: 'Thread title' });
  await expect(field).toHaveValue('Rename me please');

  // Escape cancels and leaves the title alone.
  await field.fill('Discarded name');
  await field.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(sidebar.locator('li.row', { hasText: 'Rename me please' })).toHaveCount(1);

  // Empty titles are refused inline.
  await row.getByRole('button', { name: 'Thread actions' }).click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();
  await field.fill('   ');
  await expect(dialog.getByRole('button', { name: 'Rename' })).toBeDisabled();
  await expect(dialog).toContainText('Enter a title.');

  // Enter saves the trimmed title; the sidebar shows it.
  await field.fill('  Planning the release  ');
  await field.press('Enter');
  await expect(dialog).toHaveCount(0);
  await expect(sidebar.locator('li.row', { hasText: 'Planning the release' })).toHaveCount(1);
  await expect(sidebar.getByRole('button', { name: /Rename me please/ })).toHaveCount(0);

  // Right-click still opens the menu, and Rename is there too.
  await sidebar.getByRole('button', { name: /Planning the release/ }).click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Rename' })).toBeVisible();
  await page.keyboard.press('Escape');
});
