// The panel's entry: wait for the UI kernel's bridge, then mount the Svelte component.
import { connectPanel } from '@kumikit/ui-sdk/panel';
import { mount } from 'svelte';

import Panel from './Panel.svelte';

const ctx = await connectPanel();
mount(Panel, { target: document.getElementById('app')!, props: { ctx } });
