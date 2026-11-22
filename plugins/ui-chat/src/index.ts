/**
 * kumikit.ui-chat: the chat view of a thread and renderers for every kernel item kind.
 * Built with the public UI SDK only.
 */
import type { Plugin, PluginContext, RendererProps, TranscriptItem } from '@kumikit/ui-sdk';
import { type Component, mount, unmount } from 'svelte';

import Approval from './Approval.svelte';
import Chat from './Chat.svelte';
import FileChange from './FileChange.svelte';
import Message from './Message.svelte';
import PluginEvent from './PluginEvent.svelte';
import Reasoning from './Reasoning.svelte';
import Tool from './Tool.svelte';
import { linkStyles } from './style';

type Of<K extends TranscriptItem['kind']> = Extract<TranscriptItem, { kind: K }>;

function show<P extends Record<string, unknown>>(component: Component<P>, el: HTMLElement, props: P): () => void {
  const c = mount(component, { target: el, props });
  return () => void unmount(c);
}

function renderers(ctx: PluginContext): [string, (el: HTMLElement, p: RendererProps) => () => void][] {
  return [
    ['message', (el, p) => show(Message, el, { ctx, text: (p.item as Of<'message'>).text, done: (p.item as Of<'message'>).done })],
    ['reasoning', (el, p) => show(Reasoning, el, { text: (p.item as Of<'reasoning'>).text })],
    ['tool', (el, p) => show(Tool, el, { item: p.item as Of<'tool'> })],
    ['file.changed', (el, p) => {
      const i = p.item as Of<'file.changed'>;
      return show(FileChange, el, i.diff === undefined ? { path: i.path } : { path: i.path, diff: i.diff });
    }],
    ['approval', (el, p) => show(Approval, el, { item: p.item as Of<'approval'>, run: p.run })],
    ['plugin.event', (el, p) => {
      const i = p.item as Of<'plugin.event'>;
      return show(PluginEvent, el, { plugin: i.plugin, event: i.event, data: i.data });
    }],
  ];
}

let unlink: (() => void) | undefined;

export default {
  activate(ctx) {
    unlink = linkStyles();
    ctx.slots.register('thread.view', {
      id: 'chat',
      mount: (el, props) => show(Chat, el, { ctx, address: props.address, connection: props.connection }),
    });
    for (const [kind, render] of renderers(ctx)) {
      ctx.slots.register('transcript.renderer', { kind, mount: render });
    }
  },
  deactivate() {
    // The UI kernel undoes the registrations; the stylesheet is ours to remove.
    unlink?.();
    unlink = undefined;
  },
} satisfies Plugin;
