/** kumikit.ui-settings: the client's Appearance settings page, with the public UI SDK. */
import type { Plugin } from '@kumikit/ui-sdk';
import { mount, unmount } from 'svelte';

import Appearance from './Appearance.svelte';
import Schedules from './Schedules.svelte';
import { linkStyles } from './style';

let unlink: (() => void) | undefined;

export default {
  activate(ctx) {
    unlink = linkStyles();
    ctx.slots.register('settings.page', {
      id: 'appearance',
      title: 'Appearance',
      order: 0,
      mount(el) {
        const c = mount(Appearance, { target: el, props: { ctx } });
        return () => void unmount(c);
      },
    });
    ctx.slots.register('settings.page', {
      id: 'schedules',
      title: 'Schedules',
      order: 50,
      mount(el) {
        const c = mount(Schedules, { target: el, props: { ctx } });
        return () => void unmount(c);
      },
    });
  },
  deactivate() {
    unlink?.();
    unlink = undefined;
  },
} satisfies Plugin;
