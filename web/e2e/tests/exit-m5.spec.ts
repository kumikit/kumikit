/**
 * M5 headline exit test in the browser build (E6, docs/plans/2026-11-19-m5-plugins.md):
 * From a fixture registry, the user installs a tool plugin (`acme.tool`), an ACP agent
 * granted `tools` (`acme.agent`) and a sandboxed UI panel (`acme.panel` requiring
 * `acme.tool`). In a workspace thread, the agent calls `acme_tool__echo` through the real
 * MCP tool gateway, the answer appears in the transcript, and the sandboxed panel loads.
 */
import { cpSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { ROOT, command, expect, open, test, chooseAgent } from './fixtures';

test.use({ registry: true });

test('registry tool plugin, gateway MCP turn and sandboxed panel work together', async ({ page, kk }) => {
  page.on('console', (m) => {
    if (m.type() === 'error' || m.type() === 'warning') console.log(`PAGE console ${m.type()}: ${m.text()}`);
  });
  page.on('pageerror', (e) => console.log(`PAGE error: ${e.message}`));

  const r = kk.registry;
  const transcript = join(kk.work, 'm5-gateway.jsonl');
  writeFileSync(
    transcript,
    [
      { from: 'client', message: { jsonrpc: '2.0', id: 1, method: 'initialize', params: {} } },
      { from: 'agent', message: { jsonrpc: '2.0', id: 1, result: { protocolVersion: 1 } } },
      { from: 'client', message: { jsonrpc: '2.0', id: 2, method: 'session/new', params: {} } },
      { from: 'agent', message: { jsonrpc: '2.0', id: 2, result: { sessionId: 's1' } } },
      { from: 'client', message: { jsonrpc: '2.0', id: 3, method: 'session/prompt', params: {} } },
      { from: 'mcp', session: 's1', call: { name: 'acme_tool__echo', arguments: { text: 'gateway-m5-ok' } } },
      { from: 'agent', message: { jsonrpc: '2.0', id: 3, result: { stopReason: 'end_turn' } } },
    ]
      .map((l) => JSON.stringify(l))
      .join('\n') + '\n',
  );

  // Stage a signed UI panel requiring `acme.tool@^0.1` from the built test panel.
  const panelDir = join(kk.work, 'acme-panel');
  cpSync(join(ROOT, 'target/bundle/test-ui-plugins/test.ui-panel'), panelDir, { recursive: true });
  const panelManifest = readFileSync(join(panelDir, 'plugin.toml'), 'utf8')
    .replace('id = "test.ui-panel"', 'id = "acme.panel"\ndescription = "A sandboxed panel requiring acme.tool."\nlicense = "MIT"\nrepository = "https://example.com/acme/panel"')
    .replace('name = "Panel test"', 'name = "Acme Panel"')
    .replace('title = "Panel test"', 'title = "Acme Panel"')
    + '\n[requires]\ncore = ["acme.tool@^0.1"]\n';
  writeFileSync(join(panelDir, 'plugin.toml'), panelManifest);

  await r.publish(await r.tool('0.1.0'));
  await r.publish(await r.agent('0.1.0', transcript));
  await r.publish(panelDir);
  await r.starter('acme.tool', 'acme.agent', 'acme.panel');
  await r.build();

  await open(page, kk);
  await page.getByRole('region', { name: 'Welcome to Kumikit' }).getByRole('button', { name: 'Browse the starter pack' }).click();
  const starter = page.getByRole('region', { name: 'Starter pack' });

  // Install core tool plugin.
  const toolItem = starter.getByRole('listitem').filter({ hasText: 'acme.tool' });
  await toolItem.getByRole('button', { name: /Install on/ }).click();
  const toolDialog = page.getByRole('dialog', { name: /Enable Acme Tool/ });
  await toolDialog.getByRole('button', { name: 'Grant and enable' }).click();
  await expect(page.getByRole('region', { name: 'On this core' }).getByRole('listitem').filter({ hasText: 'acme.tool' })).toContainText('Enabled');

  // Install ACP agent plugin (granted `tools`).
  const agentItem = starter.getByRole('listitem').filter({ hasText: 'acme.agent' });
  await agentItem.getByRole('button', { name: /Install on/ }).click();
  const agentDialog = page.getByRole('dialog', { name: /Enable Acme Agent/ });
  await expect(agentDialog).toContainText('tools');
  await agentDialog.getByRole('button', { name: 'Grant and enable' }).click();
  await expect(page.getByRole('region', { name: 'On this core' }).getByRole('listitem').filter({ hasText: 'acme.agent' })).toContainText('Enabled');

  // Install sandboxed UI panel (no trust prompt).
  const panelItem = starter.getByRole('listitem').filter({ hasText: 'acme.panel' });
  await panelItem.getByRole('button', { name: 'Install in this app' }).click();
  await expect(page.getByRole('region', { name: 'On this client' }).getByRole('listitem').filter({ hasText: 'acme.panel' })).toContainText('Enabled');
  await page.getByRole('button', { name: 'Close', exact: true }).click();

  // Open workspace and run an agent turn that calls `acme_tool__echo` through the gateway.
  await kk.pick(kk.project);
  await page.getByRole('navigation', { name: 'Workspaces', exact: true }).getByRole('button', { name: 'Open folder…' }).click();
  await expect(page.getByRole('navigation', { name: 'Open workspaces' }).getByRole('button', { name: 'proj', exact: true })).toBeVisible();
  await chooseAgent(page, 'Acme Agent');

  await page.getByLabel('Message').fill('use the tool');
  await page.getByLabel('Message').press('Enter');
  await expect(page.getByRole('paragraph').filter({ hasText: 'gateway-m5-ok' })).toBeVisible({ timeout: 20_000 });

  // The sandboxed panel unlocked by `requires.core = ["acme.tool@^0.1"]` is mounted and ready.
  const frame = page.frameLocator('iframe[title="Acme Panel"]');
  await expect(frame.locator('#out[data-ready="1"]')).toBeVisible({ timeout: 15_000 });
});
