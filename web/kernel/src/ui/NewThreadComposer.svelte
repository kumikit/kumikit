<!-- Inline new-thread composer: shown directly inside a workspace pane whenever no
     thread is open (including new splits and New Thread), with clipboard/file attachments,
     and Agent, Model, and Permission Mode controls directly underneath the composer. -->
<script lang="ts">
  import type { ModelInfo, PermissionMode } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import { untrack } from 'svelte';
  import type { Kernel } from '../app.svelte';
  import type { CoreConnection } from '../connections/connection.svelte';
  import {
    filesFromClipboard,
    formatBytes,
    fromPickedAttachments,
    readAttachments,
    toProtocolAttachments,
    type PendingAttachment,
  } from './attachments';
  import BranchPicker from './BranchPicker.svelte';
  import PermissionModePicker from './PermissionModePicker.svelte';

  let {
    kernel,
    connection,
    workspaceId,
    workspaceName,
    worktreePath,
    paneId,
  }: {
    kernel: Kernel;
    connection: CoreConnection;
    workspaceId: string;
    workspaceName: string | undefined;
    worktreePath?: string | undefined;
    paneId: string;
  } = $props();

  const worktreeLabel = $derived.by(() => {
    if (!worktreePath) return '';
    const parts = worktreePath.split(/[\\/]/).filter(Boolean);
    return parts[parts.length - 1] ?? worktreePath;
  });

  const providers = $derived(connection.providers);
  const workspaceRoot = $derived(connection.workspaces.find((w) => w.id === workspaceId)?.root);
  let provider = $state(untrack(() => kernel.settings.value.lastProvider ?? ''));
  let model = $state(untrack(() => kernel.settings.value.lastModel ?? ''));
  let permissionMode = $state<PermissionMode>(untrack(() => kernel.settings.value.defaultPermissionMode));
  let permissionOverridden = $state(false);
  let models = $state.raw<ModelInfo[]>([]);
  let loadingModels = $state(false);
  let text = $state('');
  let attachments = $state.raw<PendingAttachment[]>([]);
  let dragging = $state(false);
  let busy = $state(false);
  let textarea: HTMLTextAreaElement | undefined = $state();
  let fileInput: HTMLInputElement | undefined = $state();

  const fallbackInputId = `kk-draft-input-${untrack(() => paneId)}`;
  let inputId = $state(fallbackInputId);

  // Claim `#kk-chat-input` after DOM commit so any outgoing tab's composer has already
  // unmounted, while keeping every split pane's textarea id unique.
  $effect(() => {
    void kernel.activePaneId;
    if (!textarea || typeof document === 'undefined') return;
    const existing = document.getElementById('kk-chat-input');
    inputId = !existing || existing === textarea ? 'kk-chat-input' : fallbackInputId;
  });

  // Keep a valid provider selected once providers arrive from the core.
  $effect(() => {
    if (providers.length === 0) {
      provider = '';
      return;
    }
    if (!providers.some((p) => p.id === provider)) {
      const remembered = kernel.settings.value.lastProvider;
      provider = (remembered && providers.some((p) => p.id === remembered) ? remembered : providers[0]?.id) ?? '';
    }
  });

  // Load models whenever the selected provider changes.
  $effect(() => {
    const p = provider;
    models = [];
    if (!p) {
      loadingModels = false;
      model = '';
      return;
    }
    loadingModels = true;
    connection.models(p).then(
      (m) => {
        if (provider !== p) return;
        loadingModels = false;
        models = m;
        const remembered = kernel.settings.value.lastModel;
        model = (remembered && m.some((x) => x.id === remembered) ? remembered : m[0]?.id) ?? '';
      },
      () => {
        if (provider !== p) return;
        loadingModels = false;
      },
    );
  });

  // Follow the Settings default permission mode until the user overrides it for this draft.
  $effect(() => {
    const def = kernel.settings.value.defaultPermissionMode;
    if (!permissionOverridden) permissionMode = def;
  });

  // Focus the composer and reset permissions to the configured default when starting a fresh draft thread.
  let seenDraftNonce = untrack(() => kernel.draftFocusNonce);
  $effect(() => {
    const nonce = kernel.draftFocusNonce;
    const isFresh = nonce !== seenDraftNonce;
    seenDraftNonce = nonce;
    if (kernel.activePaneId === paneId) {
      if (isFresh) {
        permissionOverridden = false;
        permissionMode = kernel.settings.value.defaultPermissionMode;
      }
      if (textarea) textarea.focus();
    }
  });

  const canSend = $derived(
    !busy && Boolean(provider) && (text.trim().length > 0 || attachments.length > 0) && connection.ready,
  );

  async function addFiles(files: readonly File[], pasted = false) {
    if (files.length === 0) return;
    const res = await readAttachments(files, attachments, pasted);
    if (res.added.length > 0) attachments = [...attachments, ...res.added];
    if (res.error) kernel.notify(res.error, 'warning');
  }

  function removeAttachment(id: string) {
    attachments = attachments.filter((a) => a.id !== id);
  }

  function onPaste(e: ClipboardEvent) {
    const files = filesFromClipboard(e);
    if (files.length === 0) return;
    const plain = e.clipboardData?.getData('text/plain') ?? '';
    if (!plain) e.preventDefault();
    void addFiles(files, true);
  }

  async function openAttachmentPicker() {
    const picked = await kernel.pickAttachments(connection.id, workspaceId, worktreePath);
    if (picked === null) {
      fileInput?.click();
      return;
    }
    if (picked.length === 0) return;
    const res = await fromPickedAttachments(picked, attachments);
    if (res.added.length > 0) attachments = [...attachments, ...res.added];
    if (res.error) kernel.notify(res.error, 'warning');
  }

  function onFilePicked(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const files = input.files ? Array.from(input.files) : [];
    input.value = '';
    void addFiles(files, false);
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    const files = e.dataTransfer?.files ? Array.from(e.dataTransfer.files) : [];
    void addFiles(files, false);
  }

  async function send() {
    const message = text.trim();
    const pending = attachments;
    if ((!message && pending.length === 0) || !provider || busy) return;
    busy = true;
    const opts: {
      permissionMode?: PermissionMode;
      attachments?: ReturnType<typeof toProtocolAttachments>;
      worktreePath?: string;
    } = {
      permissionMode,
    };
    if (pending.length > 0) opts.attachments = toProtocolAttachments(pending);
    if (worktreePath) opts.worktreePath = worktreePath;
    const created = await kernel.startThreadWithMessage(
      connection.id,
      workspaceId,
      provider,
      model || undefined,
      message,
      paneId,
      opts,
    );
    busy = false;
    if (created) {
      text = '';
      attachments = [];
    }
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
</script>

<section class="draft" aria-label={`New thread in ${workspaceName ?? 'workspace'}`}>
  <div class="inner">
    <div class="intro">
      <h1>{workspaceName ?? 'Workspace'}</h1>
      {#if worktreePath}
        <div class="wt-badge" title={worktreePath}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <rect x="2" y="2.25" width="4.75" height="4.75" rx="1" />
            <rect x="9.25" y="9" width="4.75" height="4.75" rx="1" />
            <path d="M6.75 4.6h2.75a2 2 0 0 1 2 2V9" />
          </svg>
          <span>Worktree · {worktreeLabel}</span>
          <button
            type="button"
            class="wt-clear"
            aria-label="Use root project folder"
            title="Switch back to root project folder"
            onclick={() => kernel.startDraftThread({ connection: connection.id, workspace: workspaceId }, paneId)}
          >
            ×
          </button>
        </div>
      {:else}
        <p class="kk-muted">Start a new thread — pick an agent, model, and permissions below, then type or paste.</p>
      {/if}
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form
      class="composer-shell"
      class:dragging
      onsubmit={(e) => {
        e.preventDefault();
        void send();
      }}
      ondragover={(e) => {
        e.preventDefault();
        dragging = true;
      }}
      ondragleave={() => (dragging = false)}
      ondrop={onDrop}
    >
      {#if attachments.length > 0}
        <div class="att-strip" aria-label="Pending attachments">
          {#each attachments as att (att.id)}
            <div class="att-card">
              {#if att.previewUrl}
                <img class="att-thumb" src={att.previewUrl} alt={att.name} />
              {:else}
                <span class="att-file-ico" aria-hidden="true">
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3">
                    <path d="M4 2h5.5L13 5.5V14H4V2Z" stroke-linejoin="round" />
                    <path d="M9.5 2v3.5H13" stroke-linejoin="round" />
                  </svg>
                </span>
              {/if}
              <div class="att-meta">
                <span class="att-name" title={att.name}>{att.name}</span>
                <span class="att-sub">
                  {#if att.width && att.height}
                    {att.width} × {att.height}
                  {:else}
                    {formatBytes(att.size)}
                  {/if}
                </span>
              </div>
              <button
                type="button"
                class="att-remove"
                aria-label={`Remove ${att.name}`}
                title="Remove attachment"
                onclick={() => removeAttachment(att.id)}
              >
                ×
              </button>
            </div>
          {/each}
        </div>
      {/if}

      <div class="composer-box">
        <label class="kk-sr-only" for={inputId}>Message</label>
        <textarea
          bind:this={textarea}
          id={inputId}
          rows="3"
          disabled={providers.length === 0 || busy}
          placeholder={providers.length === 0 ? 'Install an agent provider plugin to start a thread' : 'Message the agent…'}
          bind:value={text}
          onkeydown={onKey}
          onpaste={onPaste}
        ></textarea>
        <div class="box-actions">
          <input
            bind:this={fileInput}
            type="file"
            multiple
            class="kk-sr-only"
            aria-label="Attach files"
            onchange={onFilePicked}
          />
          <button
            type="button"
            class="tool-btn"
            disabled={providers.length === 0 || busy}
            aria-label="Attach file"
            title="Attach file or paste image from clipboard"
            onclick={() => void openAttachmentPicker()}
          >
            <svg width="17" height="17" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M12.5 7.2L7.3 12.4a3 3 0 0 1-4.2-4.2l5.9-5.9a2 2 0 1 1 2.8 2.8L6.2 10.7a1 1 0 1 1-1.4-1.4l4.9-4.9" />
            </svg>
          </button>
          <button
            type="submit"
            class="send-btn"
            disabled={!canSend}
            aria-label="Send"
            title="Start thread and send (Enter)"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.9"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <line x1="8" y1="12.5" x2="8" y2="3.5" />
              <polyline points="4 7.25 8 3.25 12 7.25" />
            </svg>
          </button>
        </div>
      </div>

      <div class="footer">
        {#if providers.length === 0}
          <span class="no-provider">No agent provider is installed on this core.</span>
          <button type="button" class="kk-button" onclick={() => (kernel.main = { kind: 'plugins' })}>Manage plugins</button>
        {:else}
          <div class="footer-left">
            <select
              class="picker"
              aria-label="Agent"
              title="Agent provider"
              bind:value={provider}
              disabled={busy}
            >
              {#each providers as p (p.id)}
                <option value={p.id}>{p.name}</option>
              {/each}
            </select>
            <select
              class="picker model-picker"
              aria-label="Model"
              aria-busy={loadingModels}
              title={loadingModels ? 'Loading available models…' : 'Model'}
              bind:value={model}
              disabled={loadingModels || models.length === 0 || busy}
            >
              {#if loadingModels && models.length === 0}
                <option value="">Loading models…</option>
              {:else}
                {#each models as m (m.id)}
                  <option value={m.id}>{m.label}</option>
                {/each}
              {/if}
            </select>
            <PermissionModePicker
              value={permissionMode}
              disabled={busy}
              onChange={(mode) => {
                permissionOverridden = true;
                permissionMode = mode;
              }}
            />
          </div>
          <BranchPicker
            {connection}
            {workspaceId}
            {workspaceRoot}
            {workspaceName}
            {worktreePath}
            disabled={busy}
            onNotify={(m, t) => kernel.notify(m, t)}
            onSelectWorktree={(path) => {
              const target: { connection: string; workspace: string; worktreePath?: string } = {
                connection: connection.id,
                workspace: workspaceId,
              };
              if (path) target.worktreePath = path;
              kernel.startDraftThread(target, paneId);
            }}
          />
        {/if}
      </div>
    </form>
  </div>
</section>

<style>
  .draft {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px 20px;
    overflow: auto;
  }
  .inner {
    width: 100%;
    max-width: 760px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .intro {
    text-align: center;
    display: grid;
    gap: 4px;
  }
  h1 {
    margin: 0;
    font-size: var(--kk-font-size-title, x-large);
    color: var(--kk-text-primary, CanvasText);
  }
  .intro p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .wt-badge {
    display: inline-flex;
    align-items: center;
    justify-self: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .wt-badge svg {
    color: var(--kk-accent, Highlight);
  }
  .wt-clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-muted, GrayText);
    cursor: pointer;
  }
  .wt-clear:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .composer-shell {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    box-shadow: var(--kk-shadow-card, none);
    transition: border-color var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .composer-shell.dragging {
    border-color: var(--kk-accent, Highlight);
  }
  .att-strip {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 10px 2px;
  }
  .att-card {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 220px;
    padding: 4px 8px 4px 5px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
  }
  .att-thumb {
    width: 32px;
    height: 32px;
    object-fit: cover;
    border-radius: var(--kk-radius-small, 0);
    flex: none;
  }
  .att-file-ico {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-secondary, CanvasText);
    flex: none;
  }
  .att-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .att-name {
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .att-sub {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .att-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-muted, GrayText);
    cursor: pointer;
    flex: none;
  }
  .att-remove:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .composer-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 6px 6px 0;
    padding: 10px 12px 8px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-input, Field);
    transition: border-color var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .composer-box:focus-within {
    border-color: var(--kk-border-strong, Highlight);
  }
  textarea {
    width: 100%;
    min-height: 56px;
    max-height: 220px;
    resize: vertical;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--kk-text-primary, FieldText);
    font: inherit;
    line-height: var(--kk-line-height-base, normal);
  }
  textarea:focus {
    outline: none;
  }
  .box-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
  }
  .tool-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 28px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
    cursor: pointer;
    transition: background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .tool-btn:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .tool-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .send-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-accent, Highlight);
    color: var(--kk-text-on-accent, HighlightText);
    cursor: pointer;
    flex: none;
    transition:
      background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out),
      opacity var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .send-btn:hover:not(:disabled) {
    background: var(--kk-accent-hover, Highlight);
  }
  .send-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
  }
  .footer-left {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
  }
  .picker {
    height: 24px;
    max-width: 200px;
    padding: 0 6px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-primary, FieldText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .model-picker {
    max-width: 260px;
  }
  .picker:hover:not(:disabled),
  .picker:focus-visible {
    background: var(--kk-surface-hover, ButtonFace);
    border-color: var(--kk-border-subtle, ButtonBorder);
  }
  .picker:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .no-provider {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
</style>
