<!--
  Cores inside the overlay sheet (06-client, Multiple cores): the cores this window knows,
  their state, adding one by address and pairing code (the fingerprint shown for the user
  to compare before the code is sent) or by SSH destination, and removing one.
-->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import type { CoreConnection } from '../connections/connection.svelte';
  import { asConnectError, type SshProbeResult } from '../platform/index';
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

  function statusTone(status: string): 'ok' | 'warn' | 'err' {
    if (status === 'ready') return 'ok';
    if (status === 'connecting' || status === 'reconnecting') return 'warn';
    return 'err';
  }

  type Mode = 'none' | 'pair' | 'ssh';
  let mode = $state<Mode>('none');
  let label = $state('');
  let address = $state('');
  let code = $state('');
  let destination = $state('');
  let command = $state('');
  let sshAutoSync = $state(false);
  /** Remote SSH probe shown when `kumikit` is missing or mismatched during Add SSH. */
  let sshProbe = $state.raw<SshProbeResult | undefined>(undefined);
  let syncingCore = $state<string | undefined>(undefined);
  /** The fingerprint the core showed, waiting for the user to confirm it. */
  let fingerprint = $state<string | undefined>();
  let busy = $state(false);
  let problem = $state<string | undefined>();
  let removing = $state.raw<CoreConnection | undefined>();

  const appVersion = $derived(kernel.updates.info?.currentVersion ?? kernel.connections[0]?.hello?.core.version);

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
    sshAutoSync = false;
    sshProbe = undefined;
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
    sshProbe = undefined;
    const text = label.trim() || destination.trim();
    try {
      const request: { name: string; label: string; destination: string; command?: string; autoSync?: boolean } = {
        name: nameFor(text),
        label: text,
        destination: destination.trim(),
        autoSync: sshAutoSync,
      };
      if (command.trim()) request.command = command.trim();
      const info = await kernel.platform.profiles.addSsh(request);
      kernel.addedCore(info);
      kernel.notify(`Added ${info.label}`, 'success');
      reset();
    } catch (err) {
      problem = asConnectError(err).message;
      // Probe the SSH host to offer installing a matching Kumikit binary if missing/mismatched.
      if (kernel.platform.profiles.probeSsh && destination.trim()) {
        try {
          const p = await kernel.platform.profiles.probeSsh({ destination: destination.trim() });
          if (p.supported && (p.status === 'missing' || p.status === 'mismatch')) {
            sshProbe = p;
          }
        } catch {
          // Keep the original connection error.
        }
      }
    }
    busy = false;
  }

  async function installAndAddSsh() {
    if (busy || !kernel.platform.profiles.syncSsh || !destination.trim()) return;
    busy = true;
    problem = undefined;
    try {
      const synced = await kernel.platform.profiles.syncSsh({
        destination: destination.trim(),
        ...(command.trim() ? { command: command.trim() } : {}),
        ifIdle: true,
      });
      const text = label.trim() || destination.trim();
      const effectiveCmd = command.trim() || '$HOME/.local/bin/kumikit bridge';
      const info = await kernel.platform.profiles.addSsh({
        name: nameFor(text),
        label: text,
        destination: destination.trim(),
        command: effectiveCmd,
        autoSync: sshAutoSync,
      });
      kernel.addedCore(info);
      kernel.notify(`Installed Kumikit ${synced.installedVersion} on ${info.label} and connected`, 'success');
      reset();
    } catch (err) {
      problem = asConnectError(err).message;
    }
    busy = false;
  }

  function promptSyncSsh(c: CoreConnection) {
    if (!kernel.platform.profiles.syncSsh || syncingCore) return;
    kernel.dialog = {
      kind: 'confirm',
      title: `Install matching Kumikit on ${c.label}?`,
      message: `Downloads Kumikit v${appVersion ?? 'current'} locally, verifies its Ed25519 signature, streams it over SSH into ~/.local/bin/kumikit, and restarts the remote core only if remote agents are idle.`,
      confirmLabel: 'Download & install',
      tone: 'primary',
      onConfirm: async () => {
        syncingCore = c.id;
        try {
          const res = await kernel.platform.profiles.syncSsh!({ profile: c.id, ifIdle: true });
          kernel.notify(`Installed Kumikit ${res.installedVersion} on ${c.label}`, 'success');
          c.reconnectNow();
        } catch (err) {
          kernel.notify(`SSH sync failed: ${asConnectError(err).message}`, 'danger');
        } finally {
          syncingCore = undefined;
        }
      },
    };
  }

  async function remove() {
    const c = removing;
    removing = undefined;
    if (c) await kernel.removeCore(c.id);
  }

  /** Moves focus into a form when it appears, so keyboard users land on its first field. */
  function focusFirst(node: HTMLElement) {
    node.querySelector<HTMLElement>('[data-autofocus], input, button')?.focus();
  }

  function onKey(e: KeyboardEvent, close: () => void) {
    if (!isComposing(e) && e.key === 'Escape') {
      e.stopPropagation();
      close();
    }
  }
</script>

<div class="cores">
  <section class="group" aria-labelledby="kk-connected-cores">
    <div class="head">
      <div class="head-title">
        <h2 id="kk-connected-cores">Connected cores</h2>
        <span class="meta-chip">{kernel.connections.length}</span>
      </div>
    </div>
    <ul class="list" aria-label="Cores">
      {#each kernel.connections as c (c.id)}
        {@const tone = statusTone(c.status)}
        {@const canReconnect = c.status !== 'ready' && c.status !== 'unauthorized'}
        {@const canRemove = kernel.platform.profiles.editable && c.id !== kernel.platform.home}
        {@const canSyncSsh =
          c.kind === 'ssh' &&
          Boolean(kernel.platform.profiles.syncSsh) &&
          (c.status === 'mismatch' ||
            c.status === 'reconnecting' ||
            (Boolean(appVersion) && Boolean(c.hello) && c.hello!.core.version !== appVersion))}
        <li class="card" data-core={c.id}>
          <div class="card-top">
            <div class="title-col">
              <strong>{c.label}</strong>
              <span class="sub-id">{kindText[c.kind] ?? c.kind}{#if c.hello} · {c.hello.core.name}, Kumikit {c.hello.core.version}{/if}</span>
            </div>
            <span class="badge {tone}">
              <span class="badge-dot {tone}" aria-hidden="true"></span>
              {statusText[c.status] ?? c.status}
            </span>
          </div>
          {#if c.error && c.status !== 'ready'}<p class="problem">{c.error}</p>{/if}
          {#if c.kind === 'ssh' && kernel.platform.profiles.updateSsh}
            <label class="inline-check">
              <input
                type="checkbox"
                checked={c.autoSync}
                onchange={(e) => void kernel.setCoreAutoSync(c.id, (e.currentTarget as HTMLInputElement).checked)}
              />
              <span>Auto-sync matching Kumikit version</span>
            </label>
          {/if}
          {#if canReconnect || canSyncSsh || canRemove}
            <div class="actions">
              {#if canSyncSsh}
                <button
                  type="button"
                  class="kk-button primary"
                  disabled={syncingCore === c.id}
                  onclick={() => promptSyncSsh(c)}
                >
                  {syncingCore === c.id ? 'Installing…' : 'Install matching version…'}
                </button>
              {/if}
              {#if canReconnect}
                <button type="button" class="kk-button" onclick={() => c.reconnectNow()}>Reconnect now</button>
              {/if}
              {#if canRemove}
                <button type="button" class="kk-button danger" onclick={() => (removing = c)}>Remove…</button>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  </section>

  {#if kernel.platform.profiles.editable}
    <section class="group" aria-labelledby="kk-add-core">
      <div class="head">
        <h2 id="kk-add-core">Add a core</h2>
      </div>
      <div class="add-card">
        {#if mode === 'none'}
          <p class="note kk-muted">A core on another machine: run <code>kumikit serve --listen &lt;address&gt;</code> there, then <code>kumikit pair</code> for a code. Or reach it through SSH, where <code>kumikit</code> is installed and a core runs.</p>
          <div class="actions start">
            <button type="button" class="kk-button primary" onclick={() => reset('pair')}>With a pairing code…</button>
            <button type="button" class="kk-button" onclick={() => reset('ssh')}>Through SSH…</button>
          </div>
        {:else if mode === 'pair'}
          {#if !fingerprint}
            <form onsubmit={check} aria-label="Pair with a core" use:focusFirst>
              <label>
                <span>Address <span class="kk-muted">(host:port, or the address <code>kumikit pair</code> printed)</span></span>
                <input class="kk-input" bind:value={address} required data-autofocus autocomplete="off" spellcheck="false" />
              </label>
              <label>
                <span>Pairing code</span>
                <input class="kk-input" bind:value={code} required autocomplete="off" spellcheck="false" />
              </label>
              <label>
                <span>Name <span class="kk-muted">(optional)</span></span>
                <input class="kk-input" bind:value={label} placeholder={address} />
              </label>
              {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
              <div class="actions">
                <button type="button" class="kk-button" onclick={() => reset()}>Cancel</button>
                <button type="submit" class="kk-button primary" disabled={busy}>Check the core</button>
              </div>
            </form>
          {:else}
            <div class="confirm" role="group" aria-labelledby="kk-fingerprint-title" use:focusFirst>
              <h3 id="kk-fingerprint-title">Is this the core's fingerprint?</h3>
              <p class="note kk-muted">Compare it with the fingerprint <code>kumikit pair</code> printed on the core's machine. If they differ, something else answered at this address: cancel.</p>
              <p class="fingerprint" data-fingerprint>{fingerprint}</p>
              {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
              <div class="actions">
                <button type="button" class="kk-button" onclick={() => reset()}>Cancel</button>
                <button type="button" class="kk-button primary" disabled={busy} onclick={pair}>It matches: pair</button>
              </div>
            </div>
          {/if}
        {:else}
          <form onsubmit={addSsh} aria-label="Add a core through SSH" use:focusFirst>
            <label>
              <span>SSH destination <span class="kk-muted">(a host from ~/.ssh/config, or user@host)</span></span>
              <input class="kk-input" bind:value={destination} required data-autofocus autocomplete="off" spellcheck="false" />
            </label>
            <label>
              <span>Remote command <span class="kk-muted">(optional; default <code>kumikit bridge</code>)</span></span>
              <input class="kk-input" bind:value={command} autocomplete="off" spellcheck="false" />
            </label>
            <label>
              <span>Name <span class="kk-muted">(optional)</span></span>
              <input class="kk-input" bind:value={label} placeholder={destination} />
            </label>
            <label class="inline-check">
              <input type="checkbox" bind:checked={sshAutoSync} />
              <span>Automatically keep Kumikit in sync on this SSH host</span>
            </label>
            <p class="note kk-muted">The app runs <code>ssh</code> without a terminal, so it cannot ask for a password or a new host key: connect once with <code>ssh</code> in a terminal first.</p>
            {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
            {#if sshProbe}
              <div class="sync-prompt" role="status">
                <p class="note">
                  {#if sshProbe.status === 'missing'}
                    Kumikit is not installed on <code>{destination}</code> ({sshProbe.os} {sshProbe.arch}).
                  {:else}
                    <code>{destination}</code> has Kumikit <code>{sshProbe.remoteVersion ?? 'unknown'}</code> ({sshProbe.os} {sshProbe.arch}); this app is <code>{sshProbe.localVersion}</code>.
                  {/if}
                  Download the verified matching binary locally and install it to <code>~/.local/bin/kumikit</code> over SSH?
                </p>
                <div class="actions start">
                  <button
                    type="button"
                    class="kk-button primary"
                    disabled={busy}
                    onclick={() => void installAndAddSsh()}
                  >
                    {busy ? 'Installing…' : `Install Kumikit v${sshProbe.localVersion} & connect`}
                  </button>
                </div>
              </div>
            {/if}
            <div class="actions">
              <button type="button" class="kk-button" onclick={() => reset()}>Cancel</button>
              <button type="submit" class="kk-button primary" disabled={busy}>Add</button>
            </div>
          </form>
        {/if}
      </div>
    </section>
  {/if}
</div>

{#if removing}
  {@const c = removing}
  <div class="scrim" role="presentation" onclick={() => (removing = undefined)}></div>
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
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 20px 24px 28px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .head-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    color: var(--kk-text-primary, CanvasText);
  }
  h3 {
    margin: 0;
    font-size: var(--kk-font-size-base, medium);
    color: var(--kk-text-primary, CanvasText);
  }
  .meta-chip {
    padding: 1px 8px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    color: var(--kk-text-muted, GrayText);
    font-size: var(--kk-font-size-small, small);
  }
  .note {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    line-height: var(--kk-line-height-base, normal);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 10px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .card-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }
  .title-col {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .sub-id {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    overflow-wrap: anywhere;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 1px 8px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    white-space: nowrap;
    flex: none;
  }
  .badge.ok {
    background: var(--kk-state-success-tint, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .badge.warn {
    background: var(--kk-state-warning-tint, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .badge.err {
    background: var(--kk-state-danger-tint, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .badge-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-text-muted, GrayText);
  }
  .badge-dot.ok {
    background: var(--kk-state-success, Highlight);
  }
  .badge-dot.warn {
    background: var(--kk-state-warning, Highlight);
  }
  .badge-dot.err {
    background: var(--kk-state-danger, Highlight);
  }
  .add-card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px 18px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  form,
  .confirm {
    display: grid;
    gap: 12px;
    max-width: 560px;
  }
  label {
    display: grid;
    gap: 4px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-primary, CanvasText);
  }
  .inline-check {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    color: var(--kk-text-secondary, CanvasText);
  }
  .sync-prompt {
    display: grid;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-raised, Canvas);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .card .actions {
    margin-top: auto;
    padding-top: 8px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .actions.start {
    justify-content: flex-start;
  }
  .fingerprint {
    margin: 0;
    padding: 10px 12px;
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-surface-code, Canvas);
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    word-spacing: 0.4em;
    overflow-wrap: anywhere;
  }
  .problem {
    margin: 0;
    padding: 6px 10px;
    border-left: 3px solid var(--kk-state-danger, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-state-danger-tint, Canvas);
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 15vh;
    left: 50%;
    z-index: 41;
    display: grid;
    gap: 12px;
    width: min(440px, 92vw);
    transform: translateX(-50%);
    padding: 18px 20px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
  .dialog p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
</style>
