<!-- The workspaces a core plugin with `permissions.workspaces = "linked"` may act in
     (03-kernel, Client interface): the core's open workspaces with a checkbox each;
     changes go to `plugin.setWorkspaces` at once. -->
<script lang="ts">
  import type { WorkspaceId } from '@kumikit/protocol';
  import type { CoreConnection } from '../connections/connection.svelte';

  let { connection, plugin, notify }: { connection: CoreConnection; plugin: string; notify: (m: string) => void } = $props();

  let linked = $state.raw<Set<WorkspaceId>>(new Set());
  let busy = $state(false);
  const uid = Math.random().toString(36).slice(2, 8);

  $effect(() => {
    const p = plugin;
    void connection
      .call('plugin.workspaces', { plugin: p })
      .then((r) => (linked = new Set(r.workspaces)))
      .catch((e: unknown) => notify(`Cannot read the linked workspaces: ${e instanceof Error ? e.message : String(e)}`));
  });

  async function toggle(id: WorkspaceId, on: boolean) {
    const next = new Set(linked);
    if (on) next.add(id);
    else next.delete(id);
    busy = true;
    try {
      await connection.call('plugin.setWorkspaces', { plugin, workspaces: [...next] });
      linked = next;
    } catch (e) {
      notify(`Cannot change the linked workspaces: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      busy = false;
    }
  }
</script>

<fieldset class="linked">
  <legend>Workspaces it may use</legend>
  {#if connection.workspaces.length === 0}
    <p class="kk-muted">Open a project folder on this core first.</p>
  {/if}
  {#each connection.workspaces as w (w.id)}
    <label class="check" for={`kk-lw-${uid}-${w.id}`}>
      <input id={`kk-lw-${uid}-${w.id}`} type="checkbox" checked={linked.has(w.id)} disabled={busy} onchange={(e) => toggle(w.id, e.currentTarget.checked)} />
      {w.name} <span class="kk-muted">{w.root}</span>
    </label>
  {/each}
</fieldset>

<style>
  .linked {
    margin: 8px 0;
    padding: 6px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-small, 0);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
</style>
