import { fileURLToPath } from 'node:url';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

// The panel SDK. Until @kumikit/ui-sdk is published to npm, point KUMIKIT_UI_SDK at the
// web/sdk folder of a Kumikit checkout.
const sdk = process.env.KUMIKIT_UI_SDK;
if (!sdk) throw new Error('set KUMIKIT_UI_SDK to the web/sdk folder of a Kumikit checkout');

// One ES module and one stylesheet; the panel's CSP allows nothing inline.
export default defineConfig({
  plugins: [svelte()],
  resolve: { alias: { '@kumikit/ui-sdk/panel': fileURLToPath(new URL('src/panel.ts', `file://${sdk.replace(/\/?$/, '/')}`)) } },
  build: {
    lib: { entry: 'src/main.ts', formats: ['es'], fileName: () => 'main.js', cssFileName: 'style' },
    outDir: 'package/dist',
    emptyOutDir: true,
    target: 'es2022',
    reportCompressedSize: false,
  },
});
