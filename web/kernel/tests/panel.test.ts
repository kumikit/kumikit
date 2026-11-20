/** What a sandboxed panel may call (07-ui-plugins, Level 2; ADR 0019). The bridge itself
 * runs in the browser suites (web/e2e/tests/panels.spec.ts). */
import { describe, expect, it } from 'vitest';

import { tokenProperty } from '@kumikit/ui-sdk/panel';
import { PANEL_SLOTS, panelMethodParams, panelRefusal, topicRefusal, type PanelScope } from '../src/plugins/panel';

const scope: PanelScope = {
  workspace: 'w1',
  threadWorkspace: (t) => ({ t1: 'w1', t2: 'w2' })[t],
  requires: ['acme.git'],
};
const noTab: PanelScope = { ...scope, workspace: undefined };

describe('the panel policy', () => {
  it('allows reads in the tab’s workspace', () => {
    for (const [m, p] of [
      ['plugin.list', {}],
      ['provider.list', {}],
      ['provider.models', { provider: 'a/b' }],
      ['workspace.list', {}],
      ['thread.list', { workspace: 'w1' }],
      ['thread.get', { thread: 't1' }],
    ] as const) {
      expect(panelRefusal(m, p, scope), m).toBeNull();
    }
  });

  it('refuses every write, and reads outside the tab’s workspace', () => {
    for (const [m, p] of [
      ['thread.send', { thread: 't1', input: { text: 'x' } }],
      ['thread.respondApproval', { approval: 'a', decision: 'allow' }],
      ['thread.create', { workspace: 'w1' }],
      ['plugin.install', { registry: { id: 'x.y' } }],
      ['plugin.enable', { plugin: 'x.y' }],
      ['registry.add', { url: 'https://x' }],
      ['auth.devices.list', {}],
      ['auth.devices.revoke', { device: 'd' }],
      ['core.shutdown', {}],
      ['workspace.open', { path: '/' }],
      ['thread.list', { workspace: 'w2' }],
      ['thread.get', { thread: 't2' }],
      ['thread.get', { thread: 'unknown' }],
      ['other.plugin/status', {}],
    ] as const) {
      expect(panelRefusal(m, p, scope), m).not.toBeNull();
    }
    expect(panelRefusal('thread.list', { workspace: 'w1' }, noTab)).not.toBeNull();
  });

  it('forwards methods of the plugins it requires', () => {
    expect(panelRefusal('acme.git/status', { path: '.' }, scope)).toBeNull();
  });

  it('follows only topics of the tab’s workspace', () => {
    expect(topicRefusal('workspace:w1', scope)).toBeNull();
    expect(topicRefusal('thread:t1', scope)).toBeNull();
    expect(topicRefusal('workspace:w2', scope)).not.toBeNull();
    expect(topicRefusal('thread:t2', scope)).not.toBeNull();
    expect(topicRefusal('approvals', scope)).not.toBeNull();
    expect(topicRefusal('thread:t1', noTab)).not.toBeNull();
  });

  it('panels go only in slots that can hold a frame', () => {
    expect([...PANEL_SLOTS].sort()).toEqual(['inspector.tab', 'settings.page', 'sidebar.section', 'view']);
  });

  it('token names become the theme’s custom properties', () => {
    expect(tokenProperty('surface.panel')).toBe('--kk-surface-panel');
    expect(tokenProperty('motion.ease.out')).toBe('--kk-motion-ease-out');
  });
});

describe('plugin method calls from panels', () => {
  it('act for the tab’s workspace whatever the panel says', () => {
    expect(panelMethodParams({ path: '.', workspace: 'other' }, 'w1')).toEqual({ path: '.', workspace: 'w1' });
    expect(panelMethodParams({ workspace: 'other' }, undefined)).toEqual({});
    expect(panelMethodParams([1], 'w1')).toEqual({ workspace: 'w1' });
  });
});
