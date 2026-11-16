import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// A UI plugin is one ES module plus a stylesheet (07-ui-plugins, Level 3). Svelte is
// bundled in: it is not part of the UI SDK (ADR 0004). Styles go to style.css, which
// the plugin links itself; the CSP forbids inline <style>.
export default defineConfig({
  plugins: [svelte()],
  build: {
    lib: { entry: 'src/index.ts', formats: ['es'], fileName: () => 'index.js', cssFileName: 'style' },
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    reportCompressedSize: false,
  },
});
