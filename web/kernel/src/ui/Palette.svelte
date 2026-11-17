<!-- The command palette: every action, from the keyboard (06-client). -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import { trapFocus } from './dialog';

  let { kernel }: { kernel: Kernel } = $props();
  let query = $state('');
  let index = $state(0);
  const results = $derived(kernel.commands.search(query, kernel.when, kernel.commandAllowed));

  function close() {
    kernel.dialog = undefined;
  }

  async function run(id: string) {
    close();
    await kernel.attempt('The command failed', () => Promise.resolve(kernel.commands.run(id)));
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      index = Math.min(index + 1, results.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      index = Math.max(index - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const cmd = results[index];
      if (cmd) void run(cmd.id);
    }
  }
</script>

<div class="scrim" role="presentation" onclick={close}></div>
<div class="palette" role="dialog" aria-modal="true" aria-label="Commands" use:trapFocus>
  <input
    class="kk-input"
    data-autofocus
    placeholder="Type a command"
    aria-label="Command"
    aria-controls="kk-palette-list"
    aria-activedescendant={results[index] ? `kk-cmd-${index}` : undefined}
    bind:value={query}
    oninput={() => (index = 0)}
    onkeydown={onKey}
  />
  <ul id="kk-palette-list" role="listbox" aria-label="Matching commands">
    {#each results as cmd, i (cmd.id)}
      <li id={`kk-cmd-${i}`} role="option" aria-selected={i === index} class:active={i === index}>
        <button type="button" tabindex="-1" onclick={() => run(cmd.id)}>
          <span>{cmd.title}</span>
          {#if cmd.keybinding}<kbd>{cmd.keybinding.replace('Mod', kernel.mac ? '⌘' : 'Ctrl')}</kbd>{/if}
        </button>
      </li>
    {:else}
      <li class="kk-muted none">No matching command</li>
    {/each}
  </ul>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .palette {
    position: fixed;
    top: 12vh;
    left: 50%;
    width: min(560px, 92vw);
    transform: translateX(-50%);
    padding: 8px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
  ul {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    max-height: 50vh;
    overflow: auto;
  }
  li button {
    display: flex;
    justify-content: space-between;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: none;
    text-align: left;
    cursor: pointer;
  }
  li.active button {
    background: var(--kk-surface-selected, Highlight);
  }
  .none {
    padding: 6px 8px;
  }
</style>
