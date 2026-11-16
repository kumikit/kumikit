import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Two builds of the same kernel (06-client): `dist/desktop` for the shell, and
// `dist/browser-test` with the test-only relay adapter for Playwright. The platform is a
// build-time constant, so each bundle contains only its own adapter.
export default defineConfig(({ mode }) => {
  const test = mode === 'browser-test';
  return {
    plugins: [svelte()],
    base: './',
    define: { __KK_PLATFORM__: JSON.stringify(test ? 'relay' : 'tauri') },
    build: {
      outDir: test ? 'dist/browser-test' : 'dist/desktop',
      emptyOutDir: true,
      target: 'es2022',
      modulePreload: false,
      reportCompressedSize: false,
    },
    server: { port: 5174, strictPort: true },
  };
});
