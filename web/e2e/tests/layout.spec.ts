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
import { readFileSync, readdirSync, statSync } from 'node:fs';
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
  await open(page, kk);

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
});
