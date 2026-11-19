import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

// One ES module, built against the panel SDK's source (this test plugin is not an npm
// workspace, like a third party's would not be).
export default defineConfig({
  resolve: {
    alias: { '@kumikit/ui-sdk/panel': fileURLToPath(new URL('../../web/sdk/src/panel.ts', import.meta.url)) },
  },
  build: {
    lib: { entry: 'src/panel.ts', formats: ['es'], fileName: () => 'panel.js' },
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    reportCompressedSize: false,
  },
});
