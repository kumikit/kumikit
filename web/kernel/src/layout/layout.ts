/**
 * Adjustable workspace layout (06-client, Layout and splits):
 * - Resizable left and right sidebars, persisted in client settings.
 * - Recursive horizontal and vertical split tree in the main area, where each leaf pane
 *   owns its own workspace tabs.
 * - Dynamic dimensions are written through a constructed stylesheet (`adoptedStyleSheets`)
 *   so the app never uses inline style attributes (CSP `style-src` without `'unsafe-inline'`).
 */
import type { TabSpec } from '../settings/settings.svelte';

export type SplitDirection = 'horizontal' | 'vertical';
export type LeftSidebarView = 'workspaces' | 'threads';

export interface LayoutLeaf {
  kind: 'leaf';
  id: string;
  tabs: TabSpec[];
  activeTab: number;
}

export interface LayoutSplit {
  kind: 'split';
  id: string;
  direction: SplitDirection;
  /** Fraction (0.15..0.85) allocated to `first`. */
  ratio: number;
  first: LayoutNode;
  second: LayoutNode;
}

export type LayoutNode = LayoutLeaf | LayoutSplit;

export const LEFT_MIN = 180;
export const LEFT_MAX = 480;
export const LEFT_DEFAULT = 256;

export const RIGHT_MIN = 220;
export const RIGHT_MAX = 600;
export const RIGHT_DEFAULT = 340;

export const RATIO_MIN = 0.15;
export const RATIO_MAX = 0.85;

export function clamp(n: number, min: number, max: number): number {
  if (!Number.isFinite(n)) return min;
  return Math.min(max, Math.max(min, n));
}

export function defaultLayout(tabs: TabSpec[] = [], activeTab = 0): LayoutLeaf {
  return { kind: 'leaf', id: 'pane-1', tabs: [...tabs], activeTab: clampTabIndex(tabs, activeTab) };
}

export function clampTabIndex(tabs: readonly TabSpec[], index: number): number {
  if (tabs.length === 0) return 0;
  return Math.max(0, Math.min(Math.trunc(index) || 0, tabs.length - 1));
}

export function firstLeaf(node: LayoutNode): LayoutLeaf {
  return node.kind === 'leaf' ? node : firstLeaf(node.first);
}

export function findLeaf(node: LayoutNode, id: string): LayoutLeaf | undefined {
  if (node.kind === 'leaf') return node.id === id ? node : undefined;
  return findLeaf(node.first, id) ?? findLeaf(node.second, id);
}

export function allLeaves(node: LayoutNode): LayoutLeaf[] {
  return node.kind === 'leaf' ? [node] : [...allLeaves(node.first), ...allLeaves(node.second)];
}

export function countLeaves(node: LayoutNode): number {
  return node.kind === 'leaf' ? 1 : countLeaves(node.first) + countLeaves(node.second);
}

export function updateLeaf(node: LayoutNode, id: string, fn: (leaf: LayoutLeaf) => LayoutLeaf): LayoutNode {
  if (node.kind === 'leaf') return node.id === id ? fn(node) : node;
  const first = updateLeaf(node.first, id, fn);
  const second = updateLeaf(node.second, id, fn);
  return first === node.first && second === node.second ? node : { ...node, first, second };
}

/** Generates a fresh id not currently present in `root`. */
export function nextNodeId(root: LayoutNode, prefix: 'pane' | 'split'): string {
  const used = new Set<string>();
  const walk = (n: LayoutNode) => {
    used.add(n.id);
    if (n.kind === 'split') {
      walk(n.first);
      walk(n.second);
    }
  };
  walk(root);
  for (let i = 1; ; i++) {
    const id = `${prefix}-${i}`;
    if (!used.has(id)) return id;
  }
}

/** Splits leaf `id` along `direction`, placing a new leaf in `second`. */
export function splitLeaf(
  root: LayoutNode,
  id: string,
  direction: SplitDirection,
): { root: LayoutNode; newPaneId: string } {
  const target = findLeaf(root, id);
  if (!target) return { root, newPaneId: id };
  const newPaneId = nextNodeId(root, 'pane');
  const splitId = nextNodeId(root, 'split');
  const active = target.tabs[clampTabIndex(target.tabs, target.activeTab)];
  // Open the same workspace in the new split WITHOUT copying the thread id, so splitting
  // starts a fresh new thread side by side rather than duplicating the same conversation.
  const second: LayoutLeaf = {
    kind: 'leaf',
    id: newPaneId,
    tabs: active ? [{ connection: active.connection, workspace: active.workspace }] : [],
    activeTab: 0,
  };
  const replace = (n: LayoutNode): LayoutNode => {
    if (n.kind === 'leaf') {
      if (n.id !== id) return n;
      return { kind: 'split', id: splitId, direction, ratio: 0.5, first: n, second };
    }
    const first = replace(n.first);
    const s = replace(n.second);
    return first === n.first && s === n.second ? n : { ...n, first, second: s };
  };
  return { root: replace(root), newPaneId };
}

/** Closes leaf `id`, collapsing its parent split. Cannot close the last leaf. */
export function closeLeaf(root: LayoutNode, id: string): { root: LayoutNode; nextActiveId: string } {
  if (root.kind === 'leaf') return { root, nextActiveId: root.id };
  const remove = (n: LayoutNode): LayoutNode | null => {
    if (n.kind === 'leaf') return n.id === id ? null : n;
    const first = remove(n.first);
    const second = remove(n.second);
    if (first === null) return second;
    if (second === null) return first;
    return first === n.first && second === n.second ? n : { ...n, first, second };
  };
  const next = remove(root) ?? root;
  return { root: next, nextActiveId: firstLeaf(next).id };
}

/** Updates a split node's ratio, clamped to `RATIO_MIN..RATIO_MAX`. */
export function setSplitRatio(root: LayoutNode, splitId: string, ratio: number): LayoutNode {
  if (root.kind === 'leaf') return root;
  const clamped = clamp(ratio, RATIO_MIN, RATIO_MAX);
  if (root.id === splitId) return { ...root, ratio: clamped };
  const first = setSplitRatio(root.first, splitId, clamped);
  const second = setSplitRatio(root.second, splitId, clamped);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

/** Parses and validates a persisted layout node, falling back to `fallbackTabs`. */
export function parseLayout(raw: unknown, fallbackTabs: TabSpec[], fallbackActiveTab: number): LayoutNode {
  const parseNode = (v: unknown, depth: number): LayoutNode | undefined => {
    if (depth > 16 || typeof v !== 'object' || v === null) return undefined;
    const o = v as Record<string, unknown>;
    if (typeof o.id !== 'string' || !/^[a-z0-9-]+$/i.test(o.id)) return undefined;
    if (o.kind === 'leaf') {
      const tabs = Array.isArray(o.tabs)
        ? (o.tabs as unknown[]).filter(
            (t): t is TabSpec =>
              typeof t === 'object' && t !== null && typeof (t as TabSpec).connection === 'string' && typeof (t as TabSpec).workspace === 'string',
          )
        : [];
      const activeTab = typeof o.activeTab === 'number' ? clampTabIndex(tabs, o.activeTab) : 0;
      return { kind: 'leaf', id: o.id, tabs, activeTab };
    }
    if (o.kind === 'split' && (o.direction === 'horizontal' || o.direction === 'vertical')) {
      const first = parseNode(o.first, depth + 1);
      const second = parseNode(o.second, depth + 1);
      if (!first || !second) return undefined;
      const ratio = typeof o.ratio === 'number' ? clamp(o.ratio, RATIO_MIN, RATIO_MAX) : 0.5;
      return { kind: 'split', id: o.id, direction: o.direction, ratio, first, second };
    }
    return undefined;
  };
  return parseNode(raw, 0) ?? defaultLayout(fallbackTabs, fallbackActiveTab);
}

/** Builds CSS rules for sidebar widths and split ratios (applied via `adoptedStyleSheets`). */
export function layoutCss(leftWidth: number, rightWidth: number, root: LayoutNode): string {
  const rules = [
    `:root { --kk-left-w: ${Math.round(clamp(leftWidth, LEFT_MIN, LEFT_MAX))}px; --kk-right-w: ${Math.round(clamp(rightWidth, RIGHT_MIN, RIGHT_MAX))}px; }`,
  ];
  const walk = (n: LayoutNode) => {
    if (n.kind === 'split') {
      const r = clamp(n.ratio, RATIO_MIN, RATIO_MAX);
      const a = (r * 100).toFixed(3);
      const b = ((1 - r) * 100).toFixed(3);
      rules.push(`[data-split="${n.id}"] > .kk-split-first { flex: ${a} 1 0%; }`);
      rules.push(`[data-split="${n.id}"] > .kk-split-second { flex: ${b} 1 0%; }`);
      walk(n.first);
      walk(n.second);
    }
  };
  walk(root);
  return rules.join('\n');
}
