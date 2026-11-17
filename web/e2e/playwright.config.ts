import { defineConfig, devices } from '@playwright/test';

// The browser-test build against a real core through test-plugins/web-relay (M2 plan,
// Tests). Before running: `node scripts/bundle.mjs` and
// `cargo build -p kumikit-core -p kumikit-web-relay` (from the repository root).
export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  timeout: 60_000,
  expect: { timeout: 15_000 },
  use: { trace: 'retain-on-failure' },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
  ],
});
