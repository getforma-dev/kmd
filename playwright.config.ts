import { defineConfig, devices } from '@playwright/test';
import path from 'node:path';

// The binary name differs per platform, and a bare "./target/release/kmd"
// is not runnable by the Windows shell — resolve it explicitly.
const BIN = path.resolve(
  __dirname,
  'target',
  'release',
  process.platform === 'win32' ? 'kmd.exe' : 'kmd',
);

// kmd runs against a fixture workspace this repo owns, so assertions never
// depend on whatever markdown happens to sit in the developer's cwd.
const WORKSPACE = path.resolve(__dirname, 'tests', 'fixtures', 'docs-workspace');

// Ports well clear of kmd's own default (4444), so the suite can run while a
// normal kmd instance is open.
export const OWNER_PORT = 4477;
export const TUNNEL_PORT = 4478;

// Hostname the tunnel server treats as its live tunnel. Chromium is told to
// resolve it to loopback, so requests carry a real tunnel Host header without
// cloudflared being involved.
export const TUNNEL_HOST = 'kmd-e2e.trycloudflare.com';

const serverCommand = (port: number) =>
  `"${BIN}" --no-open --port ${port} --force`;

const CI = !!process.env.CI;

export default defineConfig({
  testDir: './tests/e2e',
  // CI runners are slower and colder than a dev machine — the first Windows
  // port scan pays for a PowerShell start that is already warm locally.
  timeout: CI ? 60_000 : 30_000,
  retries: CI ? 2 : 0,
  // The `github` reporter emits ::error:: annotations naming the failing test
  // and line. Without it a CI failure is just "exit code 1", and the run log
  // is not readable without repository credentials.
  reporter: CI ? [['github'], ['list']] : [['list']],
  use: {
    headless: true,
    screenshot: 'only-on-failure',
    trace: CI ? 'retain-on-failure' : 'off',
  },
  webServer: [
    {
      command: serverCommand(OWNER_PORT),
      cwd: WORKSPACE,
      port: OWNER_PORT,
      timeout: CI ? 120_000 : 30_000,
      reuseExistingServer: false,
    },
    {
      // Second instance with a seeded tunnel host. `KMD_TEST_TUNNEL_HOST` only
      // makes the server believe a tunnel is live; tunnel mode is a
      // restriction, never a privilege, so this cannot grant extra access.
      command: serverCommand(TUNNEL_PORT),
      cwd: WORKSPACE,
      port: TUNNEL_PORT,
      timeout: CI ? 120_000 : 30_000,
      reuseExistingServer: false,
      env: { KMD_TEST_TUNNEL_HOST: TUNNEL_HOST },
    },
  ],
  projects: [
    {
      name: 'chromium',
      testIgnore: ['**/tunnel.spec.ts', '**/mobile.spec.ts'],
      use: { browserName: 'chromium', baseURL: `http://localhost:${OWNER_PORT}` },
    },
    {
      name: 'mobile',
      testMatch: '**/mobile.spec.ts',
      use: { ...devices['Pixel 5'], baseURL: `http://localhost:${OWNER_PORT}` },
    },
    {
      name: 'tunnel',
      testMatch: '**/tunnel.spec.ts',
      use: {
        browserName: 'chromium',
        baseURL: `http://${TUNNEL_HOST}:${TUNNEL_PORT}`,
        launchOptions: {
          args: [`--host-resolver-rules=MAP ${TUNNEL_HOST} 127.0.0.1`],
        },
      },
    },
  ],
});
