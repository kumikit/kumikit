<script lang="ts" module>
  // The view choice sticks for the session, across dialogs.
  let lastMode: 'unified' | 'split' = 'unified';
  let lastWrap = false;
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { parseUnifiedDiff, splitDiffRows, totalChanges, type ChangedFileSummary } from './transcript';

  let {
    files,
    initial,
    onclose,
  }: { files: ChangedFileSummary[]; initial?: string; onclose: () => void } = $props();

  let dialog: HTMLDialogElement | undefined = $state();
  // svelte-ignore state_referenced_locally
  let selected = $state(initial ?? files[0]?.path ?? '');
  let mode = $state<'unified' | 'split'>(lastMode);
  let wrap = $state(lastWrap);
  $effect(() => {
    lastMode = mode;
    lastWrap = wrap;
  });

  const totals = $derived(totalChanges(files));
  const file = $derived(files.find((f) => f.path === selected) ?? files[0]);
  const rows = $derived(file?.diff ? parseUnifiedDiff(file.diff) : []);
  const split = $derived(mode === 'split' ? splitDiffRows(rows) : []);
  const titleId = `kk-changes-${Math.random().toString(36).slice(2)}`;

  onMount(() => {
    const d = dialog;
    if (!d) return;
    // jsdom and old engines lack showModal; an open dialog still works there.
    if (typeof d.showModal === 'function') d.showModal();
    else d.setAttribute('open', '');
    return () => {
      if (d.open && typeof d.close === 'function') d.close();
    };
  });

  function close() {
    if (dialog?.open && typeof dialog.close === 'function') dialog.close();
    else onclose();
  }

  function onListKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const i = files.findIndex((f) => f.path === file?.path);
    const next = files[Math.max(0, Math.min(files.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))];
    if (next) {
      selected = next.path;
      (e.currentTarget as HTMLElement).querySelector<HTMLButtonElement>(`[data-path="${CSS.escape(next.path)}"]`)?.focus();
    }
  }

  function name(path: string) {
    const i = path.lastIndexOf('/');
    return i < 0 ? path : path.slice(i + 1);
  }
  function dir(path: string) {
    const i = path.lastIndexOf('/');
    return i < 0 ? '' : path.slice(0, i);
  }
</script>

<dialog
  bind:this={dialog}
  class="kk-changes"
  aria-labelledby={titleId}
  onclose={onclose}
  onclick={(e) => {
    // A click on the backdrop lands on the dialog element itself.
    if (e.target === dialog) close();
  }}
>
  <div class="frame">
    <header>
      <div class="titles">
        <h2 id={titleId}>Changed files</h2>
        <p class="sub">
          {files.length} changed {files.length === 1 ? 'file' : 'files'}
          <span class="add">+{totals.added}</span>
          <span class="del">−{totals.removed}</span>
        </p>
      </div>
      <button type="button" class="icon-btn" aria-label="Close" title="Close (Escape)" onclick={close}>
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
          <path d="M4 4l8 8M12 4l-8 8" />
        </svg>
      </button>
    </header>
    <div class="body">
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <ul class="files" aria-label="Files" onkeydown={onListKey}>
        {#each files as f (f.path)}
          <li>
            <button
              type="button"
              class="file"
              class:selected={f.path === file?.path}
              aria-current={f.path === file?.path ? 'true' : undefined}
              data-path={f.path}
              title={f.path}
              onclick={() => (selected = f.path)}
            >
              <span class="fname">{name(f.path)}</span>
              {#if dir(f.path)}<span class="fdir">{dir(f.path)}</span>{/if}
              <span class="fstat"><span class="add">+{f.added}</span> <span class="del">−{f.removed}</span></span>
            </button>
          </li>
        {/each}
      </ul>
      <section class="diff" aria-label={file ? `Changes to ${file.path}` : 'Changes'}>
        {#if file}
          <div class="diff-head">
            <code class="path">{file.path}</code>
            <div class="toggles" role="group" aria-label="Diff view">
              <button type="button" class="seg" aria-pressed={mode === 'unified'} onclick={() => (mode = 'unified')}>Unified</button>
              <button type="button" class="seg" aria-pressed={mode === 'split'} onclick={() => (mode = 'split')}>Split</button>
              <button type="button" class="seg" aria-pressed={wrap} onclick={() => (wrap = !wrap)}>Wrap</button>
            </div>
          </div>
          {#if rows.length === 0}
            <p class="empty">The agent reported this change without a diff.</p>
          {:else if mode === 'unified'}
            <div class="table-wrap">
              <table class="unified" class:wrap>
                <tbody>
                  {#each rows as r, i (i)}
                    {#if r.kind === 'hunk'}
                      <tr class="hunk"><td colspan="3">{r.text}</td></tr>
                    {:else}
                      <tr class={r.kind}>
                        <td class="ln">{r.oldNo ?? ''}</td>
                        <td class="ln">{r.newNo ?? ''}</td>
                        <td class="code"><span class="sign">{r.kind === 'add' ? '+' : r.kind === 'del' ? '-' : ' '}</span>{r.text}</td>
                      </tr>
                    {/if}
                  {/each}
                </tbody>
              </table>
            </div>
          {:else}
            <div class="table-wrap">
              <table class="split" class:wrap>
                <colgroup><col class="lncol" /><col /><col class="lncol" /><col /></colgroup>
                <tbody>
                  {#each split as r, i (i)}
                    {#if r.kind === 'hunk'}
                      <tr class="hunk"><td colspan="4">{r.text}</td></tr>
                    {:else}
                      <tr>
                        <td class="ln {r.left?.kind ?? 'gap'}">{r.left?.oldNo ?? ''}</td>
                        <td class="code {r.left?.kind ?? 'gap'}">{r.left?.text ?? ''}</td>
                        <td class="ln {r.right?.kind ?? 'gap'}">{r.right?.newNo ?? ''}</td>
                        <td class="code {r.right?.kind ?? 'gap'}">{r.right?.text ?? ''}</td>
                      </tr>
                    {/if}
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        {/if}
      </section>
    </div>
  </div>
</dialog>

<style>
  .kk-changes {
    width: min(1280px, calc(100vw - 48px));
    height: min(860px, calc(100vh - 48px));
    max-width: none;
    max-height: none;
    padding: 0;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-primary, CanvasText);
    box-shadow: var(--kk-shadow-overlay, none);
    font-family: var(--kk-font-ui, sans-serif);
    font-size: var(--kk-font-size-base, medium);
  }
  .kk-changes::backdrop {
    background: color-mix(in srgb, var(--kk-surface-overlay, Canvas) calc(var(--kk-overlay-opacity, 0.6) * 100%), transparent);
    backdrop-filter: blur(var(--kk-overlay-blur, 0px));
  }
  .frame {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 16px 18px 12px;
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .sub {
    margin: 4px 0 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .add {
    color: var(--kk-diff-added, CanvasText);
  }
  .del {
    color: var(--kk-diff-removed, CanvasText);
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-secondary, ButtonText);
    cursor: pointer;
  }
  .icon-btn:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, ButtonText);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 14px;
    padding: 0 18px 18px;
  }
  .files {
    flex: none;
    width: 260px;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .file {
    display: grid;
    grid-template-columns: 1fr;
    width: 100%;
    padding: 6px 10px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-primary, ButtonText);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .file:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .file.selected {
    background: var(--kk-surface-selected, Highlight);
  }
  .fname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
  }
  .fdir {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .fstat {
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
  }
  .diff {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-code-background, Canvas);
    overflow: hidden;
  }
  .diff-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px 6px 12px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-card, Canvas);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .toggles {
    display: flex;
    gap: 2px;
  }
  .seg {
    padding: 3px 10px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, ButtonText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .seg:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .seg[aria-pressed='true'] {
    border-color: var(--kk-border-subtle, ButtonBorder);
    background: var(--kk-surface-raised, ButtonFace);
    color: var(--kk-text-primary, ButtonText);
  }
  .empty {
    margin: 0;
    padding: 16px;
    color: var(--kk-text-muted, GrayText);
    font-size: var(--kk-font-size-small, small);
  }
  .table-wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
    line-height: var(--kk-line-height-base, normal);
    color: var(--kk-code-text, CanvasText);
  }
  table.split {
    table-layout: fixed;
  }
  td {
    padding: 0 8px;
    vertical-align: top;
    white-space: pre;
  }
  table.wrap td.code {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  table.split td.code {
    overflow: hidden;
  }
  table.split:not(.wrap) td.code {
    text-overflow: ellipsis;
  }
  .ln {
    width: 1%;
    min-width: 3ch;
    text-align: right;
    color: var(--kk-text-muted, GrayText);
    user-select: none;
  }
  table.split .lncol {
    width: 6ch;
  }
  .sign {
    display: inline-block;
    width: 2ch;
    user-select: none;
    color: var(--kk-text-muted, GrayText);
  }
  tr.add td,
  td.add {
    background: var(--kk-diff-added-tint, Canvas);
  }
  tr.add .code,
  td.code.add,
  tr.add .sign {
    color: var(--kk-diff-added, CanvasText);
  }
  tr.del td,
  td.del {
    background: var(--kk-diff-removed-tint, Canvas);
  }
  tr.del .code,
  td.code.del,
  tr.del .sign {
    color: var(--kk-diff-removed, CanvasText);
  }
  td.gap {
    background: var(--kk-surface-card, Canvas);
  }
  tr.hunk td {
    padding: 4px 8px;
    background: var(--kk-accent-tint, Canvas);
    color: var(--kk-text-muted, GrayText);
  }
  @media (max-width: 720px) {
    .body {
      flex-direction: column;
    }
    .files {
      width: auto;
      max-height: 30%;
    }
  }
</style>
