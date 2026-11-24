/**
 * M6 layout & inspector E2E tests (06-client, Layout and splits):
 * - Resizable & collapsible left sidebar (tree vs. flat threads view), persisted across reload.
 * - Inline new-thread composer with Agent & Model selection underneath the composer (no popup).
 * - Splitting opens a fresh new thread in the new pane rather than duplicating the active thread.
 * - Top bar holds sidebar toggles & split buttons; status bar holds plugins/settings/theme/commands
 *   on the bottom-left and connection status on the bottom-right.
 * - Resizable right inspector dock with plugin dropdown (opens new tab or focuses existing tab)
 *   and closable tabs, passing WCAG AA axe checks in dark and light themes.
 */
import AxeBuilder from '@axe-core/playwright';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';

import { ROOT, command, expect, mod, open, test } from './fixtures';
import { writeKkp } from './kkp';

const PANEL = join(ROOT, 'target/bundle/test-ui-plugins/test.ui-panel');

function files(dir: string, root = dir): Record<string, string> {
  const out: Record<string, string> = {};
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) Object.assign(out, files(p, root));
    else out[relative(root, p).split('\\').join('/')] = readFileSync(p, 'utf8');
  }
  return out;
}

test('sidebars resize and collapse, splits start new threads, and right inspector dropdown manages plugin tabs', async ({ page, kk }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  // Turn the temp project into a local Git repo with two branches so branch/worktree UI is exercised.
  const gitEnv = {
    ...process.env,
    GIT_AUTHOR_NAME: 'Test',
    GIT_AUTHOR_EMAIL: 'test@example.com',
    GIT_COMMITTER_NAME: 'Test',
    GIT_COMMITTER_EMAIL: 'test@example.com',
  };
  execFileSync('git', ['init', '-b', 'main'], { cwd: kk.project, env: gitEnv });
  writeFileSync(join(kk.project, 'README.md'), 'hello\n');
  execFileSync('git', ['add', 'README.md'], { cwd: kk.project, env: gitEnv });
  execFileSync('git', ['commit', '-m', 'init'], { cwd: kk.project, env: gitEnv });
  execFileSync('git', ['branch', 'feature-a'], { cwd: kk.project, env: gitEnv });

  await open(page, kk);

  // Verify top bar layout: left sidebar toggle + New chat button, without redundant Open project folder / New thread buttons.
  const topbar = page.locator('header.topbar');
  await expect(topbar.getByRole('button', { name: 'New chat' })).toBeVisible();
  await expect(topbar.getByRole('button', { name: 'Open project folder' })).toHaveCount(0);
  await expect(topbar.getByRole('button', { name: 'New thread' })).toHaveCount(0);

  // Verify core icon uses the connected state styling.
  await expect(page.locator('[data-core-status="connected"]')).toBeVisible();

  // Verify status bar layout: plugins, settings, theme, commands on left; Connected on right;
  // no sidebar toggles or duplicate Changes/Commands buttons in footer.
  const footer = page.locator('footer');
  await expect(footer.getByRole('button', { name: 'Manage plugins' })).toBeVisible();
  await expect(footer.getByRole('button', { name: 'Open settings' })).toBeVisible();
  await expect(footer.getByRole('button', { name: 'Command palette' })).toBeVisible();
  await expect(footer.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
  await expect(footer.getByRole('button', { name: /Toggle (left|right)/ })).toHaveCount(0);

  // Install a sandboxed inspector.tab panel (repurposed from test.ui-panel).
  const panelFiles = files(PANEL);
  panelFiles['plugin.toml'] = panelFiles['plugin.toml']!
    .replace('id = "test.ui-panel"', 'id = "community.inspector-panel"')
    .replace('name = "Panel test"', 'name = "Inspector test"')
    .replace('slot = "sidebar.section"', 'slot = "inspector.tab"')
    .replace('title = "Panel test"', 'title = "Inspector test"');
  await kk.pick(writeKkp(join(kk.work, 'inspector.kkp'), panelFiles));
  await footer.getByRole('button', { name: 'Manage plugins' }).click();
  await page.getByRole('region', { name: 'On this client' }).getByRole('button', { name: 'Install from file…' }).click();
  await expect(page.getByRole('region', { name: 'On this client' }).getByRole('listitem').filter({ hasText: 'community.inspector-panel' })).toContainText('Enabled');

  // Install provider and open workspace: inline new-thread composer appears immediately (no modal).
  await kk.pick(kk.provider('claude-text'));
  await page.getByRole('region', { name: /On this core/ }).getByRole('button', { name: 'Install from folder…' }).click();
  await page.getByRole('button', { name: 'Grant and enable' }).click();
  // Close Plugins overlay by clicking outside on the backdrop scrim.
  await page.locator('[data-overlay-scrim]').click({ position: { x: 8, y: 8 } });
  await expect(page.locator('[data-overlay="plugins"]')).toHaveCount(0);

  // Verify Settings overlay opens and closes cleanly via its Close button.
  await footer.getByRole('button', { name: 'Open settings' }).click();
  await expect(page.locator('[data-overlay="settings"]')).toBeVisible();
  await page.locator('[data-overlay="settings"]').getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.locator('[data-overlay="settings"]')).toHaveCount(0);

  // Verify Cores overlay opens from the status bar and closes by clicking outside on the scrim.
  await footer.getByRole('status').filter({ hasText: 'Connected' }).click();
  await expect(page.locator('[data-overlay="cores"]')).toBeVisible();
  await page.locator('[data-overlay-scrim]').click({ position: { x: 8, y: 8 } });
  await expect(page.locator('[data-overlay="cores"]')).toHaveCount(0);

  // Start a standalone Chat directly from the top bar without selecting a project folder.
  await topbar.getByRole('button', { name: 'New chat' }).click();
  await expect(page.getByRole('heading', { name: 'New chat' })).toBeVisible();
  // Chats do not show a Git branch/worktree picker.
  await expect(page.getByRole('button', { name: /^Git (branch|worktree):/ })).toHaveCount(0);
  await page.getByLabel('Message').fill('Quick chat hello');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText('hello from kumikit');
  await expect(page.locator('.kind-pill', { hasText: 'Chat' })).toBeVisible();
  // Clean up the Chats tab so subsequent workspace assertions run against `proj`.
  await page.getByRole('button', { name: 'Close Chats' }).click();

  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true })).toBeVisible();
  await expect(page.getByLabel('Agent')).toBeVisible();
  await expect(page.getByLabel('Model')).toBeEnabled();

  // Send a message directly from the inline composer to create & run the first thread.
  await page.getByLabel('Message').fill('First thread hello');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('log', { name: 'Transcript' })).toContainText('hello from kumikit');

  // Switch left sidebar to All threads view and back.
  await page.getByRole('button', { name: 'All threads' }).click();
  await expect(page.getByRole('navigation', { name: 'Workspaces', exact: true })).toContainText('proj');
  await page.getByRole('button', { name: 'Workspaces tree' }).click();

  // Resize left sidebar with keyboard and toggle it with Mod+B.
  const leftSlider = page.getByRole('slider', { name: 'Resize left sidebar' });
  const beforeWidth = Number(await leftSlider.getAttribute('aria-valuenow'));
  await leftSlider.focus();
  await page.keyboard.press('ArrowRight');
  await expect(leftSlider).toHaveAttribute('aria-valuenow', String(beforeWidth + 16));

  await mod(page, 'b');
  await expect(page.getByRole('complementary', { name: 'Workspace sidebar' })).toHaveCount(0);
  await mod(page, 'b');
  await expect(page.getByRole('complementary', { name: 'Workspace sidebar' })).toBeVisible();

  // Open the right inspector dock from the top bar; close its tab, then re-open via dropdown.
  await page.getByRole('button', { name: 'Toggle right inspector' }).click();
  const inspector = page.getByRole('complementary', { name: 'Inspector' });
  await expect(inspector).toBeVisible();
  const frame = page.frameLocator('iframe[title="Inspector test"]');
  await expect(frame.locator('#out[data-ready="1"]')).toBeVisible({ timeout: 15_000 });

  await inspector.getByRole('button', { name: 'Close Inspector test' }).click();
  await expect(inspector.getByRole('group', { name: 'Inspector panels' }).getByRole('button', { name: 'Inspector test', exact: true })).toHaveCount(0);
  await inspector.getByRole('button', { name: 'Open UI plugin in inspector' }).click();
  await inspector.getByRole('menuitem', { name: /Inspector test/ }).click();
  await expect(inspector.getByRole('group', { name: 'Inspector panels' }).getByRole('button', { name: 'Inspector test', exact: true })).toBeVisible();
  await expect(frame.locator('#out[data-ready="1"]')).toBeVisible({ timeout: 15_000 });
  // Selecting an already-open plugin from the menu focuses it without creating a duplicate tab.
  await inspector.getByRole('button', { name: 'Open UI plugin in inspector' }).click();
  await inspector.getByRole('menuitem', { name: /Inspector test/ }).click();
  await expect(inspector.getByRole('group', { name: 'Inspector panels' }).getByRole('button', { name: 'Inspector test', exact: true })).toHaveCount(1);

  // Split right (horizontal): the new pane starts a fresh draft thread (inline composer),
  // rather than duplicating the existing thread transcript.
  await page.getByRole('button', { name: 'Split pane right' }).click();
  await expect(page.locator('[data-pane]')).toHaveCount(2);
  await expect(page.locator('[data-pane]').first().getByRole('log', { name: 'Transcript' })).toContainText('hello from kumikit');
  await expect(page.locator('[data-pane]').nth(1).getByLabel('Agent')).toBeVisible();
  await expect(footer).toContainText('2 panes');

  // Split down (vertical) as well.
  await page.getByRole('button', { name: 'Split pane down' }).click();
  await expect(page.locator('[data-pane]')).toHaveCount(3);
  await expect(footer).toContainText('3 panes');

  // WCAG AA axe check with splits + both sidebars open in dark and light themes.
  for (const theme of ['Theme: Kumikit Dark', 'Theme: Kumikit Light']) {
    await command(page, theme);
    const r = await new AxeBuilder({ page: page as never }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    expect(r.violations.map((v) => `${theme}: ${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)).toEqual([]);
  }

  // Reload and confirm sidebars, widths, open inspector tab, and split layout persisted.
  await page.reload();
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
  await expect(page.locator('[data-pane]')).toHaveCount(3);
  await expect(page.getByRole('complementary', { name: 'Inspector' })).toBeVisible();
  await expect(page.getByRole('slider', { name: 'Resize left sidebar' })).toHaveAttribute('aria-valuenow', String(beforeWidth + 16));

  // Close a split pane.
  await page.locator('[data-pane]').last().getByRole('button', { name: 'Close split pane' }).click();
  await expect(page.locator('[data-pane]')).toHaveCount(2);

  // Composer footer: new threads default to Standard (configured in Settings -> General);
  // switch to Accept edits, attach a file, and confirm it persists on the thread.
  const firstPane = page.locator('[data-pane]').first();
  await expect(firstPane.getByRole('button', { name: 'Permissions: Standard' })).toBeVisible();
  await firstPane.getByRole('button', { name: 'Permissions: Standard' }).click();
  await firstPane.getByRole('menuitemradio', { name: /Accept edits/ }).click();
  await expect(firstPane.getByRole('button', { name: 'Permissions: Accept edits' })).toBeVisible();

  const fileInput = firstPane.locator('input[type="file"]');
  await fileInput.setInputFiles({
    name: 'sample.txt',
    mimeType: 'text/plain',
    buffer: Buffer.from('hello attachment'),
  });
  await expect(firstPane.getByText('sample.txt')).toBeVisible();
  await firstPane.locator('textarea').fill('Inspect attached note');
  await firstPane.locator('textarea').press('Enter');
  await expect(firstPane.getByText('Inspect attached note')).toBeVisible();
  await expect(firstPane.getByText('sample.txt')).toBeVisible();
  await expect(firstPane.getByRole('button', { name: 'Permissions: Accept edits' })).toBeVisible();

  // Settings -> General: switch default permission mode to Full access and set Kumikit home folder.
  const customHome = join(kk.work, 'kk-home');
  await footer.getByRole('button', { name: 'Open settings' }).click();
  const settingsSheet = page.locator('[data-overlay="settings"]');
  await expect(settingsSheet.getByRole('heading', { name: 'General' })).toBeVisible();
  await settingsSheet.getByRole('radio', { name: /Full access/ }).check();
  const homeBox = settingsSheet.getByRole('textbox', { name: 'Kumikit home folder' });
  await expect(homeBox).toBeEnabled();
  await homeBox.fill(customHome);
  await settingsSheet.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(settingsSheet.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await settingsSheet.getByRole('button', { name: 'Close', exact: true }).click();

  // New tasks now inherit Full access from Settings -> General.
  await page.getByRole('button', { name: 'New task in proj' }).click();
  await expect(firstPane.getByRole('button', { name: 'Permissions: Full access' })).toBeVisible();

  // Active branch pill: switch to feature-a via Other branch…, then Switch to main, then create a worktree.
  await expect(firstPane.getByRole('button', { name: 'Git branch: main' })).toBeVisible();
  await firstPane.getByRole('button', { name: 'Git branch: main' }).click();
  await expect(firstPane.getByRole('menuitem', { name: 'Copy project path' })).toBeVisible();
  await firstPane.getByRole('menuitem', { name: /Other branch/ }).click();
  await firstPane.getByRole('menuitem', { name: 'feature-a' }).click();
  await expect(firstPane.getByRole('button', { name: 'Git branch: feature-a' })).toBeVisible();

  await firstPane.getByRole('button', { name: 'Git branch: feature-a' }).click();
  await firstPane.getByRole('menuitem', { name: 'Switch to main' }).click();
  await expect(firstPane.getByRole('button', { name: 'Git branch: main' })).toBeVisible();

  await firstPane.getByRole('button', { name: 'Git branch: main' }).click();
  await firstPane.getByRole('menuitem', { name: /New thread in a worktree/ }).click();
  await firstPane.getByPlaceholder('feat/my-change').fill('feat/demo-wt');
  await firstPane.getByRole('button', { name: 'Create worktree & start thread' }).click();
  // Worktree thread stays under the parent `proj` workspace (no extra workspace added) and
  // shows the distinct worktree trigger in the composer footer.
  await expect(firstPane.getByRole('button', { name: 'Git worktree: feat/demo-wt' })).toBeVisible();
  await expect(firstPane.getByRole('heading', { name: 'proj' })).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'feat-demo-wt' })).toHaveCount(0);
  expect(existsSync(join(customHome, 'worktrees', 'proj', 'feat-demo-wt'))).toBe(true);

  // Send a message in the worktree thread and verify it appears under `proj`, then right-click
  // to Archive -> filter inside Archived section -> Unarchive -> Delete.
  await firstPane.locator('textarea').fill('Worktree hello');
  await firstPane.locator('textarea').press('Enter');
  const sidebar = page.getByRole('navigation', { name: 'Workspaces', exact: true });
  const wtThreadBtn = sidebar.getByRole('button', { name: /Worktree hello/ });
  await expect(wtThreadBtn).toBeVisible();

  await wtThreadBtn.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Archive' }).click();
  const archivedSection = sidebar.getByRole('region', { name: 'Archived threads' });
  await expect(archivedSection).toBeVisible();
  // Collapsed by default until expanded or matched by the filter.
  await expect(archivedSection.getByRole('button', { name: /Worktree hello/ })).toHaveCount(0);

  // Filtering reveals matching archived threads automatically.
  await sidebar.getByRole('button', { name: 'Filter workspaces and threads' }).click();
  await sidebar.getByRole('searchbox', { name: 'Filter workspaces or threads' }).fill('Worktree hello');
  const archivedBtn = archivedSection.getByRole('button', { name: /Worktree hello/ });
  await expect(archivedBtn).toBeVisible();
  await archivedBtn.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Unarchive' }).click();
  await sidebar.getByRole('searchbox', { name: 'Filter workspaces or threads' }).fill('');
  await expect(sidebar.getByRole('button', { name: /Worktree hello/ })).toBeVisible();

  await sidebar.getByRole('button', { name: /Worktree hello/ }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Delete' }).click();
  // Confirm dialog appears before deleting; Cancel keeps the task.
  const confirmDialog = page.getByRole('alertdialog');
  await expect(confirmDialog).toContainText('Delete task');
  await confirmDialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(confirmDialog).toHaveCount(0);
  await expect(sidebar.getByRole('button', { name: /Worktree hello/ })).toBeVisible();

  // Confirming deletes the task and removes its managed worktree folder on disk.
  await sidebar.getByRole('button', { name: /Worktree hello/ }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Delete' }).click();
  await expect(confirmDialog).toBeVisible();
  await confirmDialog.getByRole('button', { name: 'Delete task' }).click();
  await expect(sidebar.getByRole('button', { name: /Worktree hello/ })).toHaveCount(0);
  await expect.poll(() => existsSync(join(customHome, 'worktrees', 'proj', 'feat-demo-wt'))).toBe(false);

  // Create another worktree and manage/delete it from Settings -> Worktrees.
  await page.getByRole('button', { name: 'New task in proj' }).click();
  await firstPane.getByRole('button', { name: 'Git branch: main' }).click();
  await firstPane.getByRole('menuitem', { name: /New thread in a worktree/ }).click();
  await firstPane.getByPlaceholder('feat/my-change').fill('feat/manage-wt');
  await firstPane.getByRole('button', { name: 'Create worktree & start thread' }).click();
  await expect(firstPane.getByRole('button', { name: 'Git worktree: feat/manage-wt' })).toBeVisible();
  expect(existsSync(join(customHome, 'worktrees', 'proj', 'feat-manage-wt'))).toBe(true);

  await footer.getByRole('button', { name: 'Open settings' }).click();
  await settingsSheet.getByRole('button', { name: 'Worktrees', exact: true }).click();
  await expect(settingsSheet.getByRole('heading', { name: 'Worktrees' })).toBeVisible();
  const wtItem = settingsSheet.getByRole('listitem').filter({ hasText: 'feat/manage-wt' });
  await expect(wtItem).toBeVisible();
  await expect(wtItem).toContainText(/\d+(\.\d+)?\s*(B|KB|MB|GB)/);
  await wtItem.getByRole('button', { name: 'Delete worktree feat/manage-wt' }).click();
  await expect(settingsSheet.getByText('No managed worktrees')).toBeVisible();
  expect(existsSync(join(customHome, 'worktrees', 'proj', 'feat-manage-wt'))).toBe(false);
  await settingsSheet.getByRole('button', { name: 'Close', exact: true }).click();

  // Right-click in main panel offers Split horizontally and Split vertically.
  const panesBefore = await page.locator('[data-pane]').count();
  await firstPane.locator('.leaf-body').click({ button: 'right', position: { x: 24, y: 80 } });
  await expect(page.getByRole('menuitem', { name: /Split horizontally/ })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: /Split vertically/ })).toBeVisible();
  await page.getByRole('menuitem', { name: /Split horizontally/ }).click();
  await expect(page.locator('[data-pane]')).toHaveCount(panesBefore + 1);

  // Right-clicking neutral chrome suppresses the default webview context menu.
  const suppressed = await footer.evaluate((el) => {
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    el.dispatchEvent(ev);
    return ev.defaultPrevented;
  });
  expect(suppressed).toBe(true);

  // Right-clicking a project in the sidebar offers Delete with a confirmation dialog.
  await sidebar.getByRole('button', { name: /^proj/ }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Delete' }).click();
  await expect(confirmDialog).toContainText('Delete project');
  // Verify the confirmation dialog passes WCAG AA axe checks.
  const dialogAxe = await new AxeBuilder({ page: page as never }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  expect(dialogAxe.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)).toEqual([]);
  await confirmDialog.getByRole('button', { name: 'Delete project' }).click();
  await expect(sidebar.getByRole('button', { name: /^proj/ })).toHaveCount(0);
  // Project files on disk remain untouched.
  expect(existsSync(join(kk.project, 'README.md'))).toBe(true);
});
