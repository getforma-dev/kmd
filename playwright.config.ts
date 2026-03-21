import { defineConfig } from '@playwright/test';
import path from 'path';

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
    command: './forma-dev/target/release/kmd --no-open --port 4444 --force',
    port: 4444,
    timeout: 30_000,
    reuseExistingServer: false,
    // Run from the monorepo root so it discovers all 340+ files
    cwd: path.resolve(__dirname, '..'),
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
});
