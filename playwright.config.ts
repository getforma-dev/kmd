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
  webServer: {
    command: './target/release/kmd --no-open --port 4444 --force',
    port: 4444,
    timeout: 30_000,
    reuseExistingServer: false,
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
});
