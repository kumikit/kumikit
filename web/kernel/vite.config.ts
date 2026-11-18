import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Three builds of the same kernel (06-client): `dist/desktop` for the shell,
// `dist/browser` for a browser served by a core, and `dist/browser-test` with the
// test-only relay adapter for Playwright. The platform is a build-time constant, so each
// bundle contains only its own adapter.
const PLATFORMS: Record<string, 'tauri' | 'browser' | 'relay'> = { production: 'tauri', browser: 'browser', 'browser-test': 'relay' };
const OUT: Record<string, string> = { production: 'dist/desktop', browser: 'dist/browser', 'browser-test': 'dist/browser-test' };

export default defineConfig(({ mode }) => {
  return {
    plugins: [svelte()],
    base: './',
    define: { __KK_PLATFORM__: JSON.stringify(PLATFORMS[mode] ?? 'tauri') },
    build: {
      outDir: OUT[mode] ?? 'dist/desktop',
      emptyOutDir: true,
      target: 'es2022',
      modulePreload: false,
      reportCompressedSize: false,
    },
    server: { port: 5174, strictPort: true },
  };
});
