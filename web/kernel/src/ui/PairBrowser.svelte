<!-- The browser client's pairing screen (06-client, Browser client): shown while this
browser is not paired with the core serving the page. The core keeps the device token in
an HttpOnly cookie; this page never sees it. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import { asConnectError } from '../platform/index';

  let { kernel }: { kernel: Kernel } = $props();
  let code = $state('');
  let name = $state('Browser');
  let busy = $state(false);
  let problem = $state<string | undefined>();

  async function pair(e: Event) {
    e.preventDefault();
    const browser = kernel.platform.browser;
    if (!browser || busy) return;
    busy = true;
    problem = undefined;
    try {
      await browser.pair(code.trim(), name.trim() || 'Browser');
      kernel.connection.reconnectNow();
    } catch (err) {
      const c = asConnectError(err);
      problem = c.code === 'unauthorized' ? 'The core refused the code: it is wrong, used or expired. Run `kumikit pair` again.' : c.message;
    }
    busy = false;
  }

  function focusFirst(node: HTMLElement) {
    node.querySelector<HTMLElement>('input')?.focus();
  }
</script>

<div class="pair">
  <form onsubmit={pair} aria-labelledby="kk-pair-title" use:focusFirst>
    <h1 id="kk-pair-title">Pair this browser</h1>
    <p>On the core's machine, run <code>kumikit pair</code> and enter the code it shows. Check that the certificate this browser accepted has the fingerprint <code>kumikit pair</code> prints (in the address bar's site information).</p>
    <label>Pairing code
      <input class="kk-input" bind:value={code} required autocomplete="off" spellcheck="false" />
    </label>
    <label>Name of this browser
      <input class="kk-input" bind:value={name} autocomplete="off" />
    </label>
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
    <div class="actions">
      <button type="submit" class="kk-button primary" disabled={busy}>Pair</button>
    </div>
  </form>
</div>

<style>
  .pair {
    display: grid;
    place-items: center;
    height: 100%;
    padding: 24px;
  }
  form {
    display: grid;
    gap: 12px;
    width: min(480px, 100%);
    padding: 24px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
  }
  h1 {
    margin: 0;
    font-size: var(--kk-font-size-title, x-large);
  }
  label {
    display: grid;
    gap: 4px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
  }
  .problem {
    margin: 0;
    color: var(--kk-state-danger, CanvasText);
  }
</style>
