<!-- Settings → About (06-client, About): the app's version and install kind, signed update
     checks and installs (ADR 0031, ADR 0032), what each connected core uses, opt-in
     idle-gated automatic updates, and the SSH remote core auto-sync preference. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import type { AppInstallKind } from '../platform/index';
  import ResourceUsage from './ResourceUsage.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  const u = $derived(kernel.updates);
  const home = $derived(kernel.connections[0]);
  const autoUpdate = $derived(kernel.settings.value.autoUpdate);
  const autoSyncSsh = $derived(kernel.settings.value.autoSyncSsh);

  const installKindLabel: Record<AppInstallKind, string> = {
    appimage: 'Linux AppImage (in-place updates)',
    deb: 'Debian / Ubuntu package (.deb)',
    rpm: 'RPM package (.rpm)',
    macos_app: 'macOS app bundle (in-place updates)',
    windows_nsis: 'Windows installer (in-place updates)',
    unsupported: 'Manual / development build',
  };

  function formatBytes(bytes: number | undefined): string {
    if (!bytes || bytes <= 0) return '';
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }

  function formatChecked(iso: string | undefined): string {
    if (!iso) return '';
    try {
      return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    } catch {
      return iso;
    }
  }

  function installNow() {
    if (u.phase === 'waiting_idle' && u.activeWorkCount > 0) {
      kernel.dialog = {
        kind: 'confirm',
        title: 'Install update while agents are active?',
        message: `${u.activeTurns} active turn(s) and ${u.readyQueued} queued message(s) are currently pending on this computer. Installing now will stop the local core and interrupt running turns.`,
        confirmLabel: 'Stop agents and install',
        tone: 'danger',
        onConfirm: async () => {
          await u.installNow({ ifIdle: false });
        },
      };
      return;
    }
    void u.installNow({ ifIdle: false });
  }
</script>

<div class="updates">
  <div class="intro">
    <h2>About</h2>
    <p class="kk-muted">Your Kumikit version and its updates, and what each connected core uses.</p>
  </div>

  <section class="card" aria-labelledby="kk-version-heading">
    <div class="version-row">
      <div class="version-col">
        <h3 id="kk-version-heading">Version and updates</h3>
        <div class="version-badges">
          <strong class="version-num" data-app-version>
            v{u.info?.currentVersion ?? home?.hello?.core.version ?? '0.1.0'}
          </strong>
          {#if home?.hello && u.info && home.hello.core.version !== u.info.currentVersion}
            <span class="chip">Core v{home.hello.core.version}</span>
          {/if}
          {#if u.info}
            <span class="chip">{installKindLabel[u.info.installKind]}</span>
          {/if}
        </div>
        {#if u.lastCheckedAt}
          <p class="hint">Last checked at {formatChecked(u.lastCheckedAt)}.</p>
        {/if}
      </div>
      {#if u.supported}
        <button
          type="button"
          class="kk-button"
          disabled={u.phase === 'checking' || u.phase === 'downloading' || u.phase === 'applying'}
          onclick={() => void u.checkNow()}
        >
          {u.phase === 'checking' ? 'Checking…' : 'Check for updates'}
        </button>
      {/if}
    </div>

    {#if !u.supported}
      <p class="hint">Application updates are managed from the Kumikit desktop app or via <code>kumikit update</code>.</p>
    {:else if u.phase === 'up_to_date'}
      <!-- One element: `.status` is a flex row, which would split the text runs apart. -->
      <p class="status ok" role="status">
        <span>You are on the latest signed release (<code>v{u.checkResult?.latestVersion ?? u.info?.currentVersion}</code>).</span>
      </p>
    {:else if u.phase === 'available' && u.checkResult}
      <div class="status info" role="status">
        <div class="status-body">
          <strong>Update available: v{u.checkResult.latestVersion}</strong>
          {#if u.checkResult.notes}
            <p class="hint">{u.checkResult.notes}</p>
          {/if}
          <div class="status-meta">
            {#if u.checkResult.assetSize}
              <span class="hint">{formatBytes(u.checkResult.assetSize)}</span>
            {/if}
            <a class="link" href={u.checkResult.notesUrl} data-kk-link>Copy release notes link</a>
          </div>
        </div>
        <div class="status-actions">
          {#if u.checkResult.assetAvailable}
            <button type="button" class="kk-button primary" onclick={installNow}>
              {u.checkResult.canAutoInstall ? 'Download and install' : 'Download package'}
            </button>
          {/if}
        </div>
      </div>
    {:else if u.phase === 'downloading'}
      <p class="status info" role="status">Downloading and verifying signed release…</p>
    {:else if u.phase === 'waiting_idle'}
      <div class="status warn" role="status">
        <div class="status-body">
          <strong>Update v{u.downloaded?.version ?? u.checkResult?.latestVersion} is ready</strong>
          <p class="hint">
            Waiting for active agents to finish before installing
            {#if u.activeWorkCount > 0}
              ({u.activeTurns} running, {u.readyQueued} queued)
            {/if}.
          </p>
        </div>
        <div class="status-actions">
          <button type="button" class="kk-button" onclick={installNow}>Install now</button>
        </div>
      </div>
    {:else if u.phase === 'ready'}
      <div class="status info" role="status">
        <div class="status-body">
          <strong>Update v{u.downloaded?.version ?? u.checkResult?.latestVersion} downloaded and verified</strong>
          <p class="hint">
            {u.downloaded?.canAutoInstall
              ? 'Ready to restart the local core and apply.'
              : 'Verified package is ready.'}
          </p>
        </div>
        <div class="status-actions">
          <button type="button" class="kk-button primary" onclick={installNow}>
            {u.downloaded?.canAutoInstall ? 'Install and restart' : 'Show install command'}
          </button>
        </div>
      </div>
    {:else if u.phase === 'applying'}
      <p class="status info" role="status">Stopping local core and installing update…</p>
    {:else if u.phase === 'applied' && u.appliedMessage}
      <div class="status ok" role="status">
        <div class="status-body">
          <strong>{u.packagePath ? 'Package downloaded' : 'Update applied'}</strong>
          <p class="hint">{u.appliedMessage}</p>
        </div>
      </div>
    {:else if u.phase === 'error' && u.error}
      <p class="err" role="alert">{u.error}</p>
    {/if}
  </section>

  <ResourceUsage {kernel} />

  {#if u.supported}
    <section class="section" aria-labelledby="kk-auto-updates-heading">
      <h3 id="kk-auto-updates-heading">Automatic updates</h3>
      <label class="toggle-card">
        <input
          type="checkbox"
          checked={autoUpdate}
          onchange={(e) => u.setAutoUpdate((e.currentTarget as HTMLInputElement).checked)}
        />
        <div class="opt-body">
          <span class="opt-title">Automatically download and install updates</span>
          <span class="opt-desc">
            Off by default. Periodically checks GitHub Releases for Ed25519-signed updates, downloads them in the
            background, and installs only after every agent on this computer is idle.
          </span>
        </div>
      </label>
    </section>

    <section class="section" aria-labelledby="kk-ssh-sync-heading">
      <h3 id="kk-ssh-sync-heading">Remote SSH cores</h3>
      <label class="toggle-card">
        <input
          type="checkbox"
          checked={autoSyncSsh}
          onchange={(e) =>
            kernel.settings.update({ autoSyncSsh: (e.currentTarget as HTMLInputElement).checked })}
        />
        <div class="opt-body">
          <span class="opt-title">Automatically install matching Kumikit on SSH hosts</span>
          <span class="opt-desc">
            Off by default. When an SSH core is on a different version, download the verified matching binary locally
            and install it over SSH once remote agents are idle. You can also toggle this per SSH core in Cores.
          </span>
        </div>
      </label>
    </section>
  {/if}
</div>

<style>
  .updates {
    display: grid;
    gap: 24px;
    max-width: 600px;
  }
  .intro {
    display: grid;
    gap: 4px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  .intro p,
  .hint {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  h3 {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-secondary, CanvasText);
  }
  .card {
    display: grid;
    gap: 12px;
    padding: 14px 16px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .version-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .version-col {
    display: grid;
    gap: 6px;
    min-width: 0;
  }
  .version-badges {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .version-num {
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-base, medium);
    color: var(--kk-text-primary, CanvasText);
  }
  .chip {
    padding: 1px 8px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin: 0;
    padding: 10px 12px;
    border-radius: var(--kk-radius-small, 0);
    border: 1px solid var(--kk-border-subtle, CanvasText);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-primary, CanvasText);
  }
  .status.ok {
    background: var(--kk-state-success-tint, Canvas);
  }
  .status.info {
    background: var(--kk-surface-raised, Canvas);
  }
  .status.warn {
    background: var(--kk-state-warning-tint, Canvas);
  }
  .status-body {
    display: grid;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .status-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
  }
  .status-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
  .link {
    color: var(--kk-accent, Highlight);
    font-size: var(--kk-font-size-small, small);
    text-decoration: underline;
  }
  .section {
    display: grid;
    gap: 8px;
  }
  .toggle-card {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    cursor: pointer;
    font-size: var(--kk-font-size-small, small);
  }
  .toggle-card:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .toggle-card input {
    margin-top: 2px;
  }
  .opt-body {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .opt-title {
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .opt-desc {
    color: var(--kk-text-secondary, CanvasText);
    line-height: var(--kk-line-height-base, normal);
  }
  .err {
    margin: 0;
    padding: 8px 10px;
    border-left: 3px solid var(--kk-state-danger, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-state-danger-tint, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-primary, CanvasText);
  }
</style>
