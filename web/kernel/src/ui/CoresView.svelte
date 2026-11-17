<!--
  Cores (06-client, Multiple cores): the cores this window knows, their state, and adding
  one by address and pairing code (the fingerprint shown for the user to compare before the
  code is sent) or by SSH destination, and removing one.
-->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import type { CoreConnection } from '../connections/connection.svelte';
  import { asConnectError } from '../platform/index';
  import { trapFocus } from './dialog';

  let { kernel }: { kernel: Kernel } = $props();

  const statusText: Record<string, string> = {
    ready: 'Connected',
    connecting: 'Connecting…',
    reconnecting: 'Reconnecting…',
    mismatch: 'Different version',
    unauthorized: 'Needs pairing',
    certificate_changed: 'Certificate changed',
  };
  const kindText: Record<string, string> = {
    local: 'This computer',
    tls: 'Paired over the network',
    ssh: 'Through SSH',
    core: 'The core serving this page',
  };

  type Mode = 'none' | 'pair' | 'ssh';
  let mode = $state<Mode>('none');
  let label = $state('');
  let address = $state('');
  let code = $state('');
  let destination = $state('');
  let command = $state('');
  /** The fingerprint the core showed, waiting for the user to confirm it. */
  let fingerprint = $state<string | undefined>();
  let busy = $state(false);
  let problem = $state<string | undefined>();
  let removing = $state.raw<CoreConnection | undefined>();

  /** A connection id from a label: lowercase letters, digits and dashes, unique here. */
  function nameFor(text: string): string {
    const base =
      text
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-+|-+$/g, '')
        .slice(0, 28) || 'core';
    const taken = new Set(kernel.connections.map((c) => c.id));
    let name = base === 'local' ? 'local-2' : base;
    for (let i = 2; taken.has(name); i++) name = `${base}-${i}`;
    return name;
  }

  function reset(next: Mode = 'none') {
    mode = next;
    label = address = code = destination = command = '';
    fingerprint = undefined;
    problem = undefined;
  }

  async function check(e: Event) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    problem = undefined;
    try {
      fingerprint = await kernel.platform.profiles.probe(address.trim());
    } catch (err) {
      problem = asConnectError(err).message;
    }
    busy = false;
  }

  async function pair() {
    if (!fingerprint || busy) return;
    busy = true;
    problem = undefined;
    const text = label.trim() || address.trim();
    try {
      const info = await kernel.platform.profiles.pair({ name: nameFor(text), label: text, address: address.trim(), fingerprint, code: code.trim() });
      kernel.addedCore(info);
      kernel.notify(`Paired with ${info.label}`, 'success');
      reset();
    } catch (err) {
      const e = asConnectError(err);
      problem = e.code === 'unauthorized' ? 'The core refused the code: it is wrong, used or expired. Run `kumikit pair` again.' : e.message;
      if (e.code === 'certificate_changed') fingerprint = undefined;
    }
    busy = false;
  }

  async function addSsh(e: Event) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    problem = undefined;
    const text = label.trim() || destination.trim();
    try {
      const request: { name: string; label: string; destination: string; command?: string } = { name: nameFor(text), label: text, destination: destination.trim() };
      if (command.trim()) request.command = command.trim();
      const info = await kernel.platform.profiles.addSsh(request);
      kernel.addedCore(info);
      kernel.notify(`Added ${info.label}`, 'success');
      reset();
    } catch (err) {
      problem = asConnectError(err).message;
    }
    busy = false;
  }

  async function remove() {
    const c = removing;
    removing = undefined;
    if (c) await kernel.removeCore(c.id);
  }

  function onKey(e: KeyboardEvent, close: () => void) {
    if (!isComposing(e) && e.key === 'Escape') close();
  }
</script>

<div class="cores">
  <h1>Cores</h1>
  <ul class="list" aria-label="Cores">
    {#each kernel.connections as c (c.id)}
      <li class="card" data-core={c.id}>
        <div class="title">
          <strong>{c.label}</strong>
          <span class="kk-muted">{kindText[c.kind] ?? c.kind}{#if c.hello} · {c.hello.core.name}, Kumikit {c.hello.core.version}{/if}</span>
          <span class="badge" class:ok={c.status === 'ready'}>{statusText[c.status] ?? c.status}</span>
        </div>
        {#if c.error && c.status !== 'ready'}<p class="kk-muted">{c.error}</p>{/if}
        <div class="actions">
          {#if c.status !== 'ready' && c.status !== 'unauthorized'}
            <button type="button" class="kk-button" onclick={() => c.reconnectNow()}>Reconnect now</button>
          {/if}
          {#if kernel.platform.profiles.editable && c.id !== kernel.platform.home}
            <button type="button" class="kk-button danger" onclick={() => (removing = c)}>Remove…</button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>

  {#if kernel.platform.profiles.editable}
    <section aria-labelledby="kk-add-core">
      <h2 id="kk-add-core">Add a core</h2>
      {#if mode === 'none'}
        <p class="kk-muted">A core on another machine: run <code>kumikit serve --listen &lt;address&gt;</code> there, then <code>kumikit pair</code> for a code. Or reach it through SSH, where <code>kumikit</code> is installed and a core runs.</p>
        <div class="actions start">
          <button type="button" class="kk-button primary" onclick={() => reset('pair')}>With a pairing code…</button>
          <button type="button" class="kk-button" onclick={() => reset('ssh')}>Through SSH…</button>
        </div>
      {:else if mode === 'pair'}
        {#if !fingerprint}
          <form onsubmit={check} aria-label="Pair with a core">
            <label>Address <span class="kk-muted">(host:port, or the address <code>kumikit pair</code> printed)</span>
              <input class="kk-input" bind:value={address} required data-autofocus autocomplete="off" spellcheck="false" />
            </label>
            <label>Pairing code
              <input class="kk-input" bind:value={code} required autocomplete="off" spellcheck="false" />
            </label>
            <label>Name <span class="kk-muted">(optional)</span>
              <input class="kk-input" bind:value={label} placeholder={address} />
            </label>
            {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
            <div class="actions">
              <button type="button" class="kk-button" onclick={() => reset()}>Cancel</button>
              <button type="submit" class="kk-button primary" disabled={busy}>Check the core</button>
            </div>
          </form>
        {:else}
          <div class="confirm" role="group" aria-labelledby="kk-fingerprint-title">
            <h3 id="kk-fingerprint-title">Is this the core's fingerprint?</h3>
            <p>Compare it with the fingerprint <code>kumikit pair</code> printed on the core's machine. If they differ, something else answered at this address: cancel.</p>
            <p class="fingerprint" data-fingerprint>{fingerprint}</p>
            {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
            <div class="actions">
              <button type="button" class="kk-button" onclick={() => reset()}>Cancel</button>
              <button type="button" class="kk-button primary" disabled={busy} onclick={pair}>It matches: pair</button>
            </div>
          </div>
        {/if}
      {:else}
        <form onsubmit={addSsh} aria-label="Add a core through SSH">
          <label>SSH destination <span class="kk-muted">(a host from ~/.ssh/config, or user@host)</span>
            <input class="kk-input" bind:value={destination} required data-autofocus autocomplete="off" spellcheck="false" />
          </label>
          <label>Remote command <span class="kk-muted">(optional; default <code>kumikit bridge</code>)</span>
            <input class="kk-input" bind:value={command} autocomplete="off" spellcheck="false" />
          </label>
          <label>Name <span class="kk-muted">(optional)</span>
            <input class="kk-input" bind:value={label} placeholder={destination} />
          </label>
          <p class="kk-muted">The app runs <code>ssh</code> without a terminal, so it cannot ask for a password or a new host key: connect once with <code>ssh</code> in a terminal first.</p>
          {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
          <div class="actions">
            <button type="button" class="kk-button" onclick={() => reset()}>Cancel</button>
            <button type="submit" class="kk-button primary" disabled={busy}>Add</button>
          </div>
        </form>
      {/if}
    </section>
  {/if}
</div>

{#if removing}
  {@const c = removing}
  <div class="scrim" role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-remove-core" tabindex="-1" use:trapFocus onkeydown={(e) => onKey(e, () => (removing = undefined))}>
    <h2 id="kk-remove-core">Remove {c.label}?</h2>
    <p>This window forgets the core{#if c.kind === 'tls'} and its token, and this computer is unpaired there if the core can be reached{/if}. Agents on that core keep running; its tabs say the core was removed.</p>
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={() => (removing = undefined)}>Cancel</button>
      <button type="button" class="kk-button danger" onclick={remove}>Remove</button>
    </div>
  </div>
{/if}

<style>
  .cores {
    height: 100%;
    overflow: auto;
    padding: 16px 24px;
  }
  h1 {
    margin: 0 0 12px;
    font-size: var(--kk-font-size-title, x-large);
  }
  h2 {
    margin: 16px 0 8px;
    font-size: var(--kk-font-size-large, large);
  }
  h3 {
    margin: 0;
    font-size: var(--kk-font-size-base, medium);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }
  .card {
    padding: 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
  }
  .title {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 8px;
  }
  .badge {
    margin-left: auto;
    color: var(--kk-text-secondary, CanvasText);
  }
  .badge.ok {
    color: var(--kk-text-primary, CanvasText);
  }
  form,
  .confirm {
    display: grid;
    gap: 12px;
    max-width: 520px;
  }
  label {
    display: grid;
    gap: 4px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions.start {
    justify-content: flex-start;
  }
  .fingerprint {
    margin: 0;
    padding: 8px;
    font-family: var(--kk-font-mono, monospace);
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    word-spacing: 0.4em;
  }
  .problem {
    margin: 0;
    color: var(--kk-state-danger, CanvasText);
  }
  .scrim {
    position: fixed;
    inset: 0;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 15vh;
    left: 50%;
    display: grid;
    gap: 12px;
    width: min(440px, 92vw);
    transform: translateX(-50%);
    padding: 16px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
</style>
