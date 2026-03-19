import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30_000,
  retries: 0,
  use: {
    baseURL: 'http://localhost:4444',
    headless: true,
    screenshot: 'only-on-failure',
  },
  // kmd binary starts the server — webServer manages the lifecycle
  webServer: {
    command: './forma-dev/target/release/kmd --no-open --port 4444',
    port: 4444,
    timeout: 30_000,
    reuseExistingServer: false,
    // Run from the monorepo root so it discovers all 340+ files
    cwd: '/Users/victorvillacis/dev/forma/getforma-dev',
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
});
