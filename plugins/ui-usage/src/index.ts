/**
 * kumikit.ui-usage: a plan limits meter in the thread header, an "Agent usage" command,
 * and the Agent usage dialog (ADR 0035). Built with the public UI SDK only; it names no
 * provider.
 */
import type { Plugin } from '@kumikit/ui-sdk';
import { mount, unmount } from 'svelte';

import Meter from './Meter.svelte';
import { acquire } from './store.svelte';
import { linkStyles } from './style';
import UsageDialog from './UsageDialog.svelte';

let unlink: (() => void) | undefined;
let closeStandalone: (() => void) | undefined;

export default {
  activate(ctx) {
    unlink = linkStyles();
    ctx.slots.register('thread.header', {
      id: 'usage',
      mount(el, props) {
        const c = mount(Meter, { target: el, props: { ctx, address: props.address, connection: props.connection } });
        return () => void unmount(c);
      },
    });
    ctx.commands.register({ id: 'usage.open', title: 'Agent usage', when: 'connection.ready' }, (args) => {
      const conn = ctx.connection;
      if (!conn) {
        ctx.notify('Connect to a core to view Agent usage.', 'info');
        return;
      }
      if (closeStandalone) return;
      const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : undefined;
      const initial =
        args && typeof args === 'object' && typeof (args as { provider?: unknown }).provider === 'string'
          ? (args as { provider: string }).provider
          : undefined;
      const [store, release] = acquire(conn);
      const host = document.createElement('div');
      document.body.append(host);
      let cleaned = false;
      const cleanup = () => {
        if (cleaned) return;
        cleaned = true;
        closeStandalone = undefined;
        void unmount(dialog);
        release();
        host.remove();
        previousFocus?.focus();
      };
      const dialog = mount(UsageDialog, {
        target: host,
        props: { ctx, store, initial, onclose: cleanup },
      });
      closeStandalone = cleanup;
    });
  },
  deactivate() {
    closeStandalone?.();
    // The UI kernel undoes the registrations; the stylesheet is ours to remove.
    unlink?.();
    unlink = undefined;
  },
} satisfies Plugin;
