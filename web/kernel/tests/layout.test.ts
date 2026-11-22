import { describe, expect, it } from 'vitest';

import {
  LEFT_DEFAULT,
  LEFT_MAX,
  LEFT_MIN,
  RATIO_MAX,
  RATIO_MIN,
  RIGHT_DEFAULT,
  allLeaves,
  closeLeaf,
  countLeaves,
  defaultLayout,
  findLeaf,
  layoutCss,
  parseLayout,
  setSplitRatio,
  splitLeaf,
} from '../src/layout/layout';
import { parseSettings } from '../src/settings/settings.svelte';

describe('layout split tree', () => {
  it('splits a leaf horizontally and vertically, starting a fresh thread in the same workspace', () => {
    const initial = defaultLayout([{ connection: 'local', workspace: 'ws1', thread: 't1' }], 0);
    const { root: hRoot, newPaneId: p2 } = splitLeaf(initial, 'pane-1', 'horizontal');
    expect(countLeaves(hRoot)).toBe(2);
    expect(hRoot.kind).toBe('split');
    expect(findLeaf(hRoot, p2)?.tabs).toEqual([{ connection: 'local', workspace: 'ws1' }]);

    const { root: vRoot, newPaneId: p3 } = splitLeaf(hRoot, p2, 'vertical');
    expect(countLeaves(vRoot)).toBe(3);
    expect(allLeaves(vRoot).map((l) => l.id)).toEqual(['pane-1', p2, p3]);
  });

  it('closes a split pane and collapses its parent split', () => {
    const initial = defaultLayout([{ connection: 'local', workspace: 'ws1' }], 0);
    const { root: split, newPaneId } = splitLeaf(initial, 'pane-1', 'horizontal');
    const { root: closed, nextActiveId } = closeLeaf(split, newPaneId);
    expect(closed.kind).toBe('leaf');
    expect(nextActiveId).toBe('pane-1');

    // Closing the only leaf leaves it intact.
    const same = closeLeaf(closed, 'pane-1');
    expect(same.root).toBe(closed);
  });

  it('clamps split ratios and renders CSP-safe CSS rules', () => {
    const initial = defaultLayout();
    const { root } = splitLeaf(initial, 'pane-1', 'horizontal');
    expect(root.kind).toBe('split');
    const splitId = root.kind === 'split' ? root.id : '';
    const updated = setSplitRatio(root, splitId, 0.99);
    expect(updated.kind === 'split' ? updated.ratio : 0).toBe(RATIO_MAX);
    const updatedLow = setSplitRatio(root, splitId, 0.01);
    expect(updatedLow.kind === 'split' ? updatedLow.ratio : 0).toBe(RATIO_MIN);

    const css = layoutCss(999, 10, updated);
    expect(css).toContain(`--kk-left-w: ${LEFT_MAX}px`);
    expect(css).toContain(`--kk-right-w: 220px`);
    expect(css).toContain(`[data-split="${splitId}"] > .kk-split-first`);
  });

  it('migrates legacy tabs settings into a default layout leaf', () => {
    const parsed = parseSettings(
      JSON.stringify({
        tabs: [{ connection: 'local', workspace: 'ws1' }],
        activeTab: 0,
      }),
    );
    expect(parsed.layout.kind).toBe('leaf');
    expect(parsed.activePaneId).toBe('pane-1');
    expect(parsed.tabs).toEqual([{ connection: 'local', workspace: 'ws1' }]);
    expect(parsed.leftSidebarOpen).toBe(true);
    expect(parsed.leftSidebarWidth).toBe(LEFT_DEFAULT);
    expect(parsed.rightSidebarOpen).toBe(false);
    expect(parsed.rightSidebarWidth).toBe(RIGHT_DEFAULT);
  });

  it('validates persisted layouts and rejects malformed nodes', () => {
    const fallback = [{ connection: 'local', workspace: 'ws1' }];
    expect(parseLayout({ kind: 'split', id: 'bad id!', direction: 'horizontal' }, fallback, 0)).toEqual(defaultLayout(fallback, 0));
    const parsed = parseSettings(JSON.stringify({ leftSidebarWidth: 10, leftSidebarView: 'threads' }));
    expect(parsed.leftSidebarWidth).toBe(LEFT_MIN);
    expect(parsed.leftSidebarView).toBe('threads');
  });
});
