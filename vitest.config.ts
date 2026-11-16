import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// One Vitest run for every package in the workspace. DOM tests opt in with
// `// @vitest-environment jsdom`.
export default defineConfig({
  plugins: [svelte()],
  resolve: { conditions: ['browser'] },
  test: {
    include: ['web/*/src/**/*.test.ts', 'web/*/tests/**/*.test.ts', 'plugins/*/src/**/*.test.ts'],
    environment: 'node',
  },
});
