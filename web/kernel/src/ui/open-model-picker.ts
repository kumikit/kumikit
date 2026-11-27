/** Opens the model picker (06-client, Model picker): used by the new-thread composer and,
 * through `ctx.ui.pickModel`, by UI plugins. One picker is open at a time. */
import type { ModelPick, ModelPickRequest } from '@kumikit/ui-sdk';
import { mount, unmount } from 'svelte';
import type { Kernel } from '../app.svelte';
import ModelPicker from './ModelPicker.svelte';

let closeOpen: (() => void) | undefined;

export function openModelPicker(kernel: Kernel, anchor: HTMLElement, request: ModelPickRequest): Promise<ModelPick | undefined> {
  closeOpen?.();
  const connection = kernel.connectionFor(request.connection);
  if (!connection) return Promise.resolve(undefined);
  return new Promise((resolve) => {
    const host = document.createElement('div');
    host.className = 'kk-model-picker-host';
    document.body.append(host);
    let finished = false;
    const finish = (pick: ModelPick | undefined) => {
      if (finished) return;
      finished = true;
      closeOpen = undefined;
      // Unmount after the current event, so the picker's own handlers complete first.
      queueMicrotask(() => {
        void unmount(component);
        host.remove();
        if (anchor.isConnected) anchor.focus();
      });
      if (pick) kernel.noteRecentModel(request.connection, pick.provider, pick.model);
      resolve(pick);
    };
    const component = mount(ModelPicker, {
      target: host,
      props: {
        providers: connection.providers,
        loadModels: (p: string) => connection.models(p),
        initial: { provider: request.provider, model: request.model, options: { ...(request.options ?? {}) } },
        recents: (kernel.settings.value.recentModels?.[request.connection] ?? []).filter((r) =>
          connection.providers.some((p) => p.id === r.provider),
        ),
        anchor,
        onDone: finish,
      },
    });
    closeOpen = () => finish(undefined);
  });
}
