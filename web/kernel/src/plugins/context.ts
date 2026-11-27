/** The `PluginContext` a UI plugin gets (07-ui-plugins, UI SDK). Everything it registers
 * is recorded in `dispose`, so disabling the plugin removes it. */
import type { ComposerProps, PluginContext, RendererProps, TranscriptItem, Unmount } from '@kumikit/ui-sdk';

import type { Kernel } from '../app.svelte';
import { renderMarkdown } from '../declarative/markdown';
import { variantKey } from '../theme/engine.svelte';
import { openModelPicker } from '../ui/open-model-picker';

/** Shown for an item no plugin renders. */
function fallback(el: HTMLElement, item: TranscriptItem): Unmount {
  const pre = document.createElement('pre');
  pre.className = 'kk-fallback';
  pre.textContent = item.kind === 'message' || item.kind === 'reasoning' ? item.text : `${item.kind}\n${JSON.stringify(item, null, 2)}`;
  el.append(pre);
  return () => pre.remove();
}

export function createContext(k: Kernel, id: string, dispose: (() => void)[]): PluginContext {
  const keep = (d: () => void) => {
    dispose.push(d);
    return d;
  };
  const storage = () => k.settings.value.pluginStorage[id] ?? {};
  const setStorage = (next: Record<string, string>) =>
    k.settings.update({ pluginStorage: { ...k.settings.value.pluginStorage, [id]: next } });
  return {
    id,
    slots: {
      register: (slot, contribution) => keep(k.slots.register(id, slot, contribution)),
    },
    commands: {
      register: (spec, handler) => keep(k.commands.register(spec, handler, id)),
      run: (cmd, args) => k.commands.run(cmd, args),
    },
    // The active tab's core (07-ui-plugins, UI SDK).
    get connection() {
      return k.connection.ready || k.connection.hello ? k.pluginConnection() : undefined;
    },
    threads: {
      watch: (address, fn) => keep(k.threads.watch(address, fn)),
    },
    theme: {
      get tokens() {
        return k.theme.tokens;
      },
      get appearance() {
        return k.theme.appearance;
      },
      get variants() {
        return k.theme.variants.map((v) => ({ key: variantKey(v), name: v.name, appearance: v.appearance }));
      },
      get choice() {
        const s = k.settings.value;
        return { variant: s.themeVariant ?? null, appearance: s.appearance, reduceMotion: s.reduceMotion };
      },
      select(choice) {
        const s = k.settings.value;
        const variant = choice.variant === undefined ? s.themeVariant : (choice.variant ?? undefined);
        const next: Partial<typeof s> = { appearance: choice.appearance ?? s.appearance, reduceMotion: choice.reduceMotion ?? s.reduceMotion };
        k.settings.update(variant === undefined ? { ...next, themeVariant: undefined } : { ...next, themeVariant: variant });
        k.applyTheme();
      },
      onChange: (fn) => keep(k.theme.onChange(fn)),
    },
    storage: {
      get: (key) => storage()[key],
      set: (key, value) => setStorage({ ...storage(), [key]: String(value) }),
      delete: (key) => {
        const { [key]: _gone, ...rest } = storage();
        setStorage(rest);
      },
    },
    notify: (message, tone) => k.notify(message, tone),
    ui: {
      markdown: (el, text) => renderMarkdown(el, text),
      renderItem(el: HTMLElement, props: RendererProps): Unmount {
        const kind = props.item.kind === 'plugin.event' ? `plugin.event:${props.item.plugin}/${props.item.event}` : props.item.kind;
        // Renderers whose plugin's requires.core this thread's core does not meet are skipped.
        const entry = k.rendererFor(kind, props.address.connection);
        if (!entry) return fallback(el, props.item);
        try {
          return entry.contribution.mount(el, props);
        } catch (e) {
          k.slots.fail(entry.plugin, e);
          el.replaceChildren();
          return fallback(el, props.item);
        }
      },
      pickModel: (anchor, request) => openModelPicker(k, anchor, request),
      renderAccessories(el: HTMLElement, props: ComposerProps): Unmount {
        const cleanups: (() => void)[] = [];
        for (const entry of k.slotsIn('composer.accessory')) {
          const wrap = el.ownerDocument.createElement('span');
          wrap.className = 'kk-composer-accessory';
          el.append(wrap);
          try {
            const disposeAcc = entry.contribution.mount(wrap, props);
            cleanups.push(() => {
              if (typeof disposeAcc === 'function') disposeAcc();
              wrap.remove();
            });
          } catch (e) {
            k.slots.fail(entry.plugin, e);
            wrap.remove();
          }
        }
        return () => {
          for (const fn of cleanups) fn();
        };
      },
    },
  };
}
