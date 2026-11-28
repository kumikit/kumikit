/**
 * M7 exit criteria in the browser build: edit-and-resend forks before the turn with its
 * prompt preloaded, `/btw` answers a side question in the inspector and inserts into the
 * main composer, `/later` adds a deferred entry in the queue panel, `/` slash menu and
 * `@` file mentions work in the composer, and Mod+F finds in the thread.
 */
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { command, openAgents, expect, mod, open, test, chooseAgent } from './fixtures';

test('M7 threads: find, slash menu, @ mentions, /later, /btw side thread, and edit fork', async ({ page, kk }) => {
  writeFileSync(join(kk.project, 'notes.md'), '# Notes\n');
  await open(page, kk);

  // Install a declarative ACP provider replaying a recorded text transcript.
  const core = await openAgents(page);
  await kk.pick(kk.provider('claude-text'));
  await core.getByRole('button', { name: 'Install from folder…' }).click();
  const grant = page.getByRole('dialog', { name: /Enable Recorded/ });
  await grant.getByRole('button', { name: 'Grant and enable' }).click();
  await expect(core.getByRole('listitem').filter({ hasText: 'test.recorded1' })).toContainText('Enabled');
  await page.getByRole('button', { name: 'Close', exact: true }).click();

  // Open the workspace and run the first turn.
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true })).toBeVisible();
  await chooseAgent(page, 'Recorded agent 1');

  const composer = page.locator('main').getByLabel('Message', { exact: true });
  await composer.fill('First prompt about woodworking');
  await composer.press('Enter');
  await expect(page.locator('main').getByText('hello from kumikit')).toBeVisible();
  await expect(page.getByText('Idle', { exact: true })).toBeVisible();

  // Find in thread (Mod+F).
  await composer.focus();
  await mod(page, 'f');
  const findBar = page.getByRole('search', { name: 'Find in thread' });
  await expect(findBar).toBeVisible();
  await findBar.getByLabel('Find in thread').fill('woodworking');
  await expect(findBar).toContainText('1 / 1');
  await page.keyboard.press('Escape');
  await expect(findBar).toBeHidden();

  // Slash command menu inserts without sending.
  await composer.fill('/la');
  const slashMenu = page.getByRole('listbox', { name: 'Slash commands' });
  await expect(slashMenu).toBeVisible();
  await slashMenu.getByRole('option').filter({ hasText: '/later' }).click();
  await expect(composer).toHaveValue('/later ');

  // `/later` adds a deferred message with its countdown chip in the queue panel.
  await composer.fill('/later 15m check the build');
  await composer.press('Enter');
  const queue = page.locator('main').getByRole('region', { name: 'Queued messages' });
  await expect(queue).toContainText('check the build');
  await expect(queue).toContainText('In 15m');

  // `@` mention picker lists workspace files and inserts `@notes.md `.
  await composer.fill('Look at @not');
  const mentionMenu = page.getByRole('listbox', { name: 'Mention file' });
  await expect(mentionMenu).toBeVisible();
  await mentionMenu.getByRole('option', { name: 'notes.md' }).click();
  await expect(composer).toHaveValue('Look at @notes.md ');

  // `/btw` creates side threads in the inspector; multiple side threads can be switched via history.
  await composer.fill('/btw first side question');
  await composer.press('Enter');
  const inspector = page.getByRole('complementary', { name: 'Inspector' });
  const insertBtn = inspector.getByRole('button', { name: 'Insert into main' });
  await expect(insertBtn).toBeVisible();
  await insertBtn.click();
  await expect(composer).toHaveValue(/hello from kumikit/);

  await composer.fill('/btw second side question');
  await composer.press('Enter');
  const sideHistory = inspector.getByLabel('Side thread history');
  await expect(sideHistory.locator('option')).toHaveCount(2);
  await expect(inspector.getByRole('log', { name: 'Transcript' })).toContainText('second side question');
  // Switch back to the first side thread via the history switcher.
  await sideHistory.selectOption({ label: 'first side question' });
  await expect(inspector.getByRole('log', { name: 'Transcript' })).toContainText('first side question');

  // Edit on the user message forks before that turn while the inspector is still open:
  // both the forked thread (with its banner) and the inspector update immediately.
  await page.locator('main').getByRole('button', { name: 'Edit prompt in fork' }).first().click();
  await expect(page.locator('main').getByRole('note')).toContainText('Forked from an earlier thread');
  await expect(page.locator('main').getByRole('note')).toContainText('Workspace files were not rewound');
  await expect(composer).toHaveValue('First prompt about woodworking');
  await expect(inspector.getByText('New side thread')).toBeVisible();
});
