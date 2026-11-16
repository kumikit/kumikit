/** Starts the UI kernel. */
import { mount } from 'svelte';

import { Kernel } from './app.svelte';
import { loadPlatform } from './platform/index';
import App from './ui/App.svelte';

const platform = await loadPlatform();
const dark = matchMedia('(prefers-color-scheme: dark)');
const kernel = new Kernel({
  platform,
  storage: localStorage,
  document,
  systemDark: () => dark.matches,
  mac: navigator.platform.startsWith('Mac') || navigator.userAgent.includes('Mac OS'),
});
dark.addEventListener('change', () => kernel.applyTheme());
const target = document.getElementById('app');
if (target) mount(App, { target, props: { kernel } });
await kernel.start();
