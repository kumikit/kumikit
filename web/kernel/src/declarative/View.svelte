<!-- Draws a declarative view (07-ui-plugins, Level 1) with the active theme. -->
<script lang="ts">
  import type { DeclarativeView, TreeNode } from '@kumikit/ui-sdk';
  import Form from './Form.svelte';
  import { renderMarkdown } from './markdown';

  let { view, run }: { view: DeclarativeView; run: (command: string, args?: unknown) => void } = $props();
  let values = $state<Record<string, unknown>>({});
  $effect(() => {
    if (view.model === 'form') values = view.values;
  });

  function markdown(el: HTMLElement, text: string) {
    renderMarkdown(el, text);
    return { update: (t: string) => renderMarkdown(el, t) };
  }
</script>

{#snippet item(label: string, detail: string | undefined, command: string | undefined, id: string)}
  {#if command}
    <button type="button" class="row" onclick={() => run(command, { id })}>{label}{#if detail}<span class="kk-muted"> {detail}</span>{/if}</button>
  {:else}
    <span class="row">{label}{#if detail}<span class="kk-muted"> {detail}</span>{/if}</span>
  {/if}
{/snippet}

{#snippet tree(nodes: TreeNode[])}
  <ul role="group">
    {#each nodes as n (n.id)}
      <li role="treeitem" aria-selected="false" aria-expanded={n.children ? true : undefined}>
        {@render item(n.label, n.detail, n.command, n.id)}
        {#if n.children}{@render tree(n.children)}{/if}
      </li>
    {/each}
  </ul>
{/snippet}

<div class="view {view.model}">
  {#if view.model === 'list'}
    <ul>
      {#each view.items as i (i.id)}<li>{@render item(i.label, i.detail, i.command, i.id)}</li>{/each}
    </ul>
  {:else if view.model === 'tree'}
    <div role="tree">{@render tree(view.items)}</div>
  {:else if view.model === 'table'}
    <table>
      <thead><tr>{#each view.columns as c (c.key)}<th scope="col">{c.title}</th>{/each}</tr></thead>
      <tbody>
        {#each view.rows as r, i (i)}
          <tr>{#each view.columns as c (c.key)}<td>{String(r[c.key] ?? '')}</td>{/each}</tr>
        {/each}
      </tbody>
    </table>
  {:else if view.model === 'markdown'}
    <div class="md" use:markdown={view.text}></div>
  {:else if view.model === 'key-value'}
    <dl>
      {#each view.entries as e (e.key)}<dt>{e.key}</dt><dd>{e.value}</dd>{/each}
    </dl>
  {:else if view.model === 'form'}
    <Form schema={view.schema} {values} onchange={(v) => (values = v)} />
    {#if view.submit}
      {@const submit = view.submit}
      <p><button type="button" class="kk-button primary" onclick={() => run(submit, values)}>Save</button></p>
    {/if}
  {:else if view.model === 'progress'}
    <label>{view.label}
      {#if view.value === undefined}<progress></progress>{:else}<progress value={view.value} max="1"></progress>{/if}
    </label>
  {/if}
</div>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  ul ul {
    padding-left: 16px;
  }
  .row {
    display: block;
    width: 100%;
    padding: 4px 6px;
    border: 0;
    background: none;
    text-align: left;
  }
  button.row {
    cursor: pointer;
    border-radius: var(--kk-radius-small, 0);
  }
  button.row:hover {
    background: var(--kk-surface-hover, Canvas);
  }
  table {
    border-collapse: collapse;
  }
  th,
  td {
    padding: 4px 10px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    text-align: left;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    margin: 0;
  }
  dt {
    color: var(--kk-text-secondary, CanvasText);
  }
  dd {
    margin: 0;
  }
  progress {
    display: block;
    width: 100%;
    accent-color: var(--kk-accent, Highlight);
  }
</style>
