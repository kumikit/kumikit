/** A lost connection: the tab says so, reconnects by itself, and the transcript has no
 * gap or duplicate afterwards. */
import { command, expect, open, test } from './fixtures';

test('reconnects and resumes the thread without duplicates', async ({ page, kk }) => {
  await open(page, kk);
  await kk.pick(kk.provider('claude-text'));
  await command(page, 'Manage plugins');
  await page.getByRole('region', { name: /On this core/ }).getByRole('button', { name: 'Install from folder…' }).click();
  await page.getByRole('button', { name: 'Grant and enable' }).click();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByLabel('Agent')).toBeVisible();
  await page.getByLabel('Message').fill('Say hello');
  await page.getByLabel('Message').press('Enter');
  const transcript = page.getByRole('log', { name: 'Transcript' });
  // The turn is over once its usage is shown.
  await expect(transcript).toContainText(' out');
  await expect(page.getByText('Idle', { exact: true })).toBeVisible();
  const before = await transcript.innerText();

  await kk.drop();
  await expect(page.getByText(/Reconnecting/).first()).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'Connected' })).toBeVisible();
  await expect.poll(() => transcript.innerText()).toBe(before);
  await expect(transcript.getByText('Say hello')).toHaveCount(1);

  // The thread still works after the reconnect.
  await page.getByLabel('Message').fill('Again');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByText('Again')).toHaveCount(1);
});
