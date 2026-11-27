/**
 * M3 in the browser build: one window with tabs on a local core and a remote one (paired
 * through the real shell code, pinned TLS and a device token). Cutting the remote's
 * network shows "reconnecting" only in its tabs while the local tabs keep streaming.
 * The agents replay recorded Claude Code transcripts.
 */
import AxeBuilder from '@axe-core/playwright';
import type { Page } from '@playwright/test';

import { command, expect, open, test, type Kumikit } from './fixtures';

test.use({ remote: true });

const sidebar = (page: Page) => page.getByRole('navigation', { name: 'Workspaces', exact: true });
const tabs = (page: Page) => page.getByRole('navigation', { name: 'Open workspaces' });
const tab = (page: Page, core: string) => tabs(page).locator(`[data-core="${core}"]`);

/** Adds the remote core from the Cores view; returns the code `kumikit pair` printed. */
async function addRemote(page: Page, kk: Kumikit) {
  const printed = await kk.remote.pairCode();
  await command(page, 'Cores: add, remove, reconnect');
  await page.getByRole('button', { name: 'With a pairing code…' }).click();
  const form = page.getByRole('form', { name: 'Pair with a core' });
  await form.getByLabel('Address').fill(printed.address);
  await form.getByLabel('Pairing code').fill(printed.code);
  await form.getByLabel('Name').fill('Dev box');
  await form.getByRole('button', { name: 'Check the core' }).click();
  // The fingerprint is shown before the code is sent, to compare with the printed one.
  await expect(page.locator('[data-fingerprint]')).toHaveText(printed.fingerprint);
  await page.getByRole('button', { name: 'It matches: pair' }).click();
  const card = page.getByRole('list', { name: 'Cores' }).getByRole('listitem').filter({ hasText: 'Dev box' });
  await expect(card).toContainText('Connected');
  await page.locator('[data-overlay="cores"]').getByRole('button', { name: 'Close', exact: true }).click();
  return printed;
}

/** Installs a provider replaying `transcript`: from a folder on the local core, by upload
 * on the remote one. */
async function installProvider(page: Page, kk: Kumikit, core: 'This computer' | 'Dev box', transcript: string) {
  await command(page, 'Manage plugins');
  const region = page.getByRole('region', { name: /On this core/ });
  await region.getByLabel('Core').selectOption({ label: core });
  if (core === 'This computer') {
    await kk.pick(kk.provider(transcript, { mode: 'default' }));
    await region.getByRole('button', { name: 'Install from folder…' }).click();
  } else {
    await expect(region.getByRole('button', { name: 'Install from folder…' })).toHaveCount(0);
    const chooser = page.waitForEvent('filechooser');
    await region.getByRole('button', { name: 'Install from file…' }).click();
    await (await chooser).setFiles(kk.providerKkp(transcript, { mode: 'default' }));
  }
  const grant = page.getByRole('dialog', { name: new RegExp(`Enable Recorded .* on ${core}`) });
  await grant.getByRole('button', { name: 'Grant and enable' }).click();
  await expect(region.getByRole('listitem').filter({ hasText: 'test.recorded' }).last()).toContainText('Enabled');
  // The recorded provider runs on provider-acp, which the core seeds after it is ready.
  await expect(region.getByRole('listitem').filter({ hasText: 'kumikit.provider-acp' })).toContainText('Enabled');
  await page.getByRole('button', { name: 'Close', exact: true }).click();
}

/** Opens a project on a core and starts a thread with its only recorded provider. */
async function startThread(page: Page, kk: Kumikit, core: 'This computer' | 'Dev box', message: string) {
  const section = sidebar(page).getByRole('region', { name: core, exact: true });
  if (core === 'This computer') {
    await kk.pick(kk.project);
    await section.getByRole('button', { name: 'Open folder…' }).click();
  } else {
    // The picker shows this computer's files, so the path on the core's machine is typed.
    await section.getByRole('button', { name: 'Open folder…' }).click();
    const dialog = page.getByRole('dialog', { name: /Open a project folder on Dev box/ });
    await dialog.getByLabel(/Path on/).fill(kk.remote.project);
    await dialog.getByRole('button', { name: 'Open' }).click();
  }
  await expect(tab(page, core === 'This computer' ? 'local' : 'dev-box')).toBeVisible();
  await expect(page.getByLabel('Agent')).toBeVisible();
  await page.getByRole('button', { name: /Permissions:/ }).click();
  await page.getByRole('menuitemradio', { name: /Standard/ }).click();
  await page.getByLabel('Message').fill(message);
  await page.getByLabel('Message').press('Enter');
}

test('tabs from two cores; cutting one core’s network leaves the other streaming', async ({ page, kk }) => {
  await open(page, kk);
  await addRemote(page, kk);
  await installProvider(page, kk, 'This computer', 'claude-tool-allow');
  await installProvider(page, kk, 'Dev box', 'claude-tool-allow');

  // The remote thread waits on an approval.
  await startThread(page, kk, 'Dev box', 'Add a line to notes.txt');
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Needs your approval');
  await expect(sidebar(page).getByRole('region', { name: 'Dev box', exact: true })).toContainText('waiting for approval');

  // Cut the remote's network. Only its tab and its sidebar group say so.
  await kk.remote.cut();
  await expect(tab(page, 'dev-box')).toContainText('reconnecting');
  await expect(sidebar(page).getByRole('region', { name: 'Dev box', exact: true })).toContainText('reconnecting');

  // Meanwhile a turn on the local core streams, asks, is allowed and finishes.
  await startThread(page, kk, 'This computer', 'Add a line to notes.txt');
  await expect(tab(page, 'local')).not.toContainText('reconnecting');
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Needs your approval');
  await page.getByRole('region', { name: 'Approval' }).getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page.getByRole('button', { name: /changed files?/ }).first()).toBeVisible();
  await expect(page.getByText('Idle', { exact: true })).toBeVisible();
  await expect(tab(page, 'dev-box')).toContainText('reconnecting');
  await expect(tab(page, 'local')).not.toContainText('reconnecting');

  // Restore the network; the remote tab comes back to the same waiting approval.
  await kk.remote.restore();
  await tab(page, 'dev-box').getByRole('button').first().click();
  const now = page.getByRole('button', { name: 'Reconnect now' });
  if (await now.isVisible()) await now.click();
  await expect(tab(page, 'dev-box')).not.toContainText('reconnecting');
  await expect(page.getByRole('region', { name: 'Approval' })).toContainText('Needs your approval');
  await expect(page.getByRole('log', { name: 'Transcript' }).getByText('Add a line to notes.txt')).toHaveCount(1);
  await page.getByRole('region', { name: 'Approval' }).getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page.getByRole('button', { name: /changed files?/ }).first()).toBeVisible();
  await expect(page.getByText('Idle', { exact: true })).toBeVisible();
});

test('a revoked device needs pairing again; a changed certificate is refused', async ({ page, kk }) => {
  await open(page, kk);
  await addRemote(page, kk);
  await sidebar(page).getByRole('region', { name: 'Dev box', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  const dialog = page.getByRole('dialog', { name: /Open a project folder on Dev box/ });
  await dialog.getByLabel(/Path on/).fill(kk.remote.project);
  await dialog.getByRole('button', { name: 'Open' }).click();
  await expect(tab(page, 'dev-box')).toBeVisible();

  await kk.remote.revoke();
  await expect(tab(page, 'dev-box')).toContainText('needs pairing');
  await expect(page.getByRole('alert').filter({ hasText: 'no longer accepts this computer' })).toBeVisible();
  // The local core is untouched.
  await expect(sidebar(page).getByRole('region', { name: 'This computer', exact: true })).not.toContainText('needs pairing');

  // Re-adding with a new code works; then the core's certificate changes.
  await command(page, 'Cores: add, remove, reconnect');
  await page.getByRole('list', { name: 'Cores' }).getByRole('listitem').filter({ hasText: 'Dev box' }).getByRole('button', { name: 'Remove…' }).click();
  await page.getByRole('dialog', { name: 'Remove Dev box?' }).getByRole('button', { name: 'Remove' }).click();
  await page.locator('[data-overlay="cores"]').getByRole('button', { name: 'Close', exact: true }).click();
  await expect(tab(page, 'dev-box')).toContainText('removed');
  await addRemote(page, kk);
  await kk.remote.rotate();
  await expect(sidebar(page).getByRole('region', { name: 'Dev box', exact: true })).toContainText('certificate changed');
});

test('a UI plugin that needs a core plugin shows only on that core', async ({ page, kk }) => {
  await open(page, kk);
  await addRemote(page, kk);
  await installProvider(page, kk, 'Dev box', 'claude-text');
  // A trusted UI plugin that needs the provider plugin just installed on the remote only.
  const { writeKkp } = await import('./kkp');
  const { join } = await import('node:path');
  await kk.pick(
    writeKkp(join(kk.work, 'needs.kkp'), {
      'plugin.toml': 'id = "community.needs"\nname = "Needs"\nversion = "1.0.0"\nkind = ["ui"]\napi = "^0.1"\n[ui]\nentry = "index.js"\n[requires]\ncore = ["test.recorded1@^0.1"]\n',
      'index.js': "export default { activate(ctx) { ctx.slots.register('statusbar.item', { id: 'needs', mount(el) { el.textContent = 'needs is here'; } }); } };",
    }),
  );
  await command(page, 'Manage plugins');
  await page.getByRole('region', { name: 'On this client' }).getByRole('button', { name: 'Install from file…' }).click();
  await page.getByRole('dialog', { name: 'Trust Needs?' }).getByRole('button', { name: 'Trust and enable' }).click();
  await page.getByRole('button', { name: 'Close', exact: true }).click();

  await kk.pick(kk.project);
  await sidebar(page).getByRole('region', { name: 'This computer', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByText('needs is here')).toHaveCount(0);
  await sidebar(page).getByRole('region', { name: 'Dev box', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  const dialog = page.getByRole('dialog', { name: /Open a project folder on Dev box/ });
  await dialog.getByLabel(/Path on/).fill(kk.remote.project);
  await dialog.getByRole('button', { name: 'Open' }).click();
  await expect(page.getByText('needs is here')).toBeVisible();
  await tab(page, 'local').getByRole('button').first().click();
  await expect(page.getByText('needs is here')).toHaveCount(0);
});

test('the Cores view has no axe violations, light and dark, and works from the keyboard', async ({ page, kk }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await open(page, kk);
  const printed = await kk.remote.pairCode();
  for (const theme of ['Theme: Kumikit Light', 'Theme: Kumikit Dark']) {
    await command(page, theme);
    await command(page, 'Cores: add, remove, reconnect');
    await page.getByRole('button', { name: 'With a pairing code…' }).click();
    const r = await new AxeBuilder({ page: page as never }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    expect(r.violations.map((v) => `${theme}: ${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)).toEqual([]);
    await page.getByRole('button', { name: 'Cancel' }).click();
  }
  // Keyboard only: the address field has focus when the form opens.
  await page.getByRole('button', { name: 'With a pairing code…' }).press('Enter');
  await page.keyboard.type(printed.address);
  await page.keyboard.press('Tab');
  await page.keyboard.type(printed.code);
  await page.keyboard.press('Enter');
  await expect(page.locator('[data-fingerprint]')).toHaveText(printed.fingerprint);
  await page.getByRole('button', { name: 'It matches: pair' }).press('Enter');
  await expect(page.getByRole('list', { name: 'Cores' }).getByRole('listitem').filter({ hasText: printed.address })).toContainText('Connected');
});
