<!--
  The confirmations for a core plugin, shared by the plugin manager and Settings → Agents
  (06-client, Plugin manager): grant its permissions and enable it, remove it, or accept an
  update that asks for more permissions.
-->
<script module lang="ts">
  import type { AvailableUpdate, InstalledPlugin } from '@kumikit/protocol';

  /** What the dialog asks about. */
  export type CorePluginAsk =
    | { kind: 'grant'; plugin: InstalledPlugin }
    | { kind: 'remove'; plugin: InstalledPlugin }
    | { kind: 'widen'; update: AvailableUpdate };
</script>

<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import { describePermissions, startsPrograms } from '../plugins/permissions';
  import { trapFocus } from './dialog';

  let {
    ask,
    core,
    onclose,
    ongrant,
    onremove,
    onupdate,
  }: {
    ask: CorePluginAsk;
    /** The label of the core it is on. */
    core: string;
    onclose: () => void;
    ongrant?: (p: InstalledPlugin) => void;
    onremove?: (p: InstalledPlugin, deleteData: boolean) => void;
    onupdate?: (u: AvailableUpdate) => void;
  } = $props();

  const publisher = (id: string) => id.split('.')[0];

  function onKey(e: KeyboardEvent) {
    if (!isComposing(e) && e.key === 'Escape') {
      e.stopPropagation();
      onclose();
    }
  }

  /** Runs the action, then closes: the action reads the plugin before `ask` is cleared. */
  function done(f: () => void) {
    f();
    onclose();
  }
</script>

<div class="scrim" role="presentation"></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-core-plugin-ask" tabindex="-1" use:trapFocus onkeydown={onKey}>
  {#if ask.kind === 'grant'}
    {@const p = ask.plugin}
    <h2 id="kk-core-plugin-ask">Enable {p.name} on {core}?</h2>
    <p class="kk-muted">{p.id} {p.version} from <strong>{publisher(p.id)}</strong>{#if p.repository} · {p.repository}{/if}</p>
    {#if p.signed && p.publisher}
      <p>Signed with key <code class="key">{p.publisher.key}</code>{#if p.publisher.registry}, the publisher’s key in the registry {p.publisher.registry}.{:else}; not checked against a registry.{/if}</p>
    {:else}
      <p class="warning">This package is not signed, so Kumikit cannot check who made it.</p>
    {/if}
    {#if startsPrograms(p.requested)}
      <p class="warning">It starts programs outside the sandbox. They run with your full user access, so they can read and change anything you can.</p>
    {/if}
    <p>It asks for these permissions, granted together:</p>
    <ul class="perms">
      {#each describePermissions(p.requested) as line (line)}<li>{line}</li>{/each}
    </ul>
    <div class="actions">
      <button type="button" class="kk-button" onclick={onclose}>Not now</button>
      <button type="button" class="kk-button primary" data-autofocus onclick={() => done(() => ongrant?.(p))}>Grant and enable</button>
    </div>
  {:else if ask.kind === 'remove'}
    {@const p = ask.plugin}
    <h2 id="kk-core-plugin-ask">Remove {p.name}?</h2>
    <p>Its running turns are cancelled. Keep its data (settings and stored state) for a later install, or delete it too.</p>
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={onclose}>Cancel</button>
      <button type="button" class="kk-button" onclick={() => done(() => onremove?.(p, false))}>Remove, keep data</button>
      <button type="button" class="kk-button danger" onclick={() => done(() => onremove?.(p, true))}>Remove and delete data</button>
    </div>
  {:else}
    {@const u = ask.update}
    <h2 id="kk-core-plugin-ask">Update {u.plugin} to {u.to}?</h2>
    <p>This version asks for more permissions than the one you granted. Until you agree, {u.plugin} stays on {u.from}. The new version:</p>
    <ul class="perms">{#each describePermissions(u.requested) as line (line)}<li>{line}</li>{/each}</ul>
    <div class="actions">
      <button type="button" class="kk-button" data-autofocus onclick={onclose}>Not now</button>
      <button type="button" class="kk-button primary" onclick={() => done(() => onupdate?.(u))}>Grant and update</button>
    </div>
  {/if}
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 12vh;
    left: 50%;
    z-index: 41;
    display: grid;
    gap: 10px;
    width: min(520px, 92vw);
    transform: translateX(-50%);
    padding: 18px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    color: var(--kk-text-primary, CanvasText);
  }
  p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .perms {
    margin: 0;
    padding-left: 18px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding-top: 4px;
  }
  .warning {
    padding: 6px 10px;
    border-left: 3px solid var(--kk-state-warning, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-state-warning-tint, Canvas);
  }
  .key {
    overflow-wrap: anywhere;
  }
</style>
