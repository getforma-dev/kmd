import { test, expect } from '@playwright/test';
import { TUNNEL_HOST, TUNNEL_PORT } from '../../playwright.config';

/**
 * Tunnel sharing is docs-only. These tests drive a kmd instance that believes a
 * tunnel is live (via `KMD_TEST_TUNNEL_HOST`) and reach it through a hostname
 * Chromium resolves to loopback, so page navigations carry a genuine tunnel
 * `Host` header — the same signal the middleware uses in production.
 *
 * The rule under test: a visitor may read documentation and nothing else.
 *
 * Note on API assertions: `page.request` / the `request` fixture do NOT honour
 * Chromium's `--host-resolver-rules`, so they must address loopback directly
 * and set `Host` themselves. Using the tunnel hostname there would send the
 * request to the real internet.
 */

const CSRF_HEADERS = { 'X-KMD-Client': '1' };
const ORIGIN = `http://127.0.0.1:${TUNNEL_PORT}`;
const AS_VISITOR = { Host: TUNNEL_HOST };

test.describe('Tunnel: docs are readable', () => {
  test('the docs page renders for a visitor', async ({ page }) => {
    await page.goto('/#docs');
    await expect(page.locator('.main-header h1')).toHaveText('Documentation');
    await expect(page.locator('.markdown-body')).not.toBeEmpty();
  });

  test('the server reports the request as a visitor', async ({ request }) => {
    const res = await request.get(`${ORIGIN}/api/tunnel`, { headers: AS_VISITOR });
    expect(res.status()).toBe(200);
    expect((await res.json()).visitor).toBe(true);
  });

  test('a localhost request is not reported as a visitor', async ({ request }) => {
    const res = await request.get(`${ORIGIN}/api/tunnel`, { headers: { Host: 'localhost' } });
    expect(res.status()).toBe(200);
    expect((await res.json()).visitor).toBe(false);
  });

  test('docs search works through the tunnel', async ({ request }) => {
    const res = await request.get(`${ORIGIN}/api/docs/search?q=KMDFIXTURE`, { headers: AS_VISITOR });
    expect(res.status()).toBe(200);
    expect((await res.json()).results.length).toBeGreaterThan(0);
  });
});

test.describe('Tunnel: everything else is blocked', () => {
  for (const path of [
    '/api/scripts',
    '/api/ports',
    '/api/processes',
    '/api/env',
    '/api/chains',
    '/api/terminal/sessions',
    '/ws/terminal',
  ]) {
    test(`GET ${path} is forbidden`, async ({ request }) => {
      const res = await request.get(`${ORIGIN}${path}`, { headers: AS_VISITOR });
      expect(res.status()).toBe(403);
    });
  }

  for (const path of ['/api/scripts/run', '/api/shell/exec', '/api/ports/scan']) {
    test(`POST ${path} is forbidden`, async ({ request }) => {
      const res = await request.post(`${ORIGIN}${path}`, {
        headers: { ...AS_VISITOR, ...CSRF_HEADERS },
        data: {},
      });
      expect(res.status()).toBe(403);
    });
  }

  test('a visitor cannot start or stop the tunnel', async ({ request }) => {
    for (const path of ['/api/tunnel/start', '/api/tunnel/stop']) {
      const res = await request.post(`${ORIGIN}${path}`, {
        headers: { ...AS_VISITOR, ...CSRF_HEADERS },
        data: {},
      });
      expect(res.status()).toBe(403);
    }
  });
});

test.describe('Tunnel: the visitor UI hides what it cannot reach', () => {
  test('only the Docs tab is offered, and forcing another route bounces back', async ({ page }) => {
    await page.goto('/#docs');
    await expect(page.locator('.nav-item')).toHaveCount(1);
    await expect(page.locator('.nav-item').first()).toContainText('Docs');

    await page.goto('/#terminal');
    await page.waitForTimeout(1000);
    expect(await page.evaluate(() => location.hash)).toBe('#docs');
  });

  test('the document is marked read-only and titled as shared', async ({ page }) => {
    await page.goto('/#docs');
    await expect(page.locator('body')).toHaveClass(/read-only/);
    expect(await page.title()).toContain('(shared)');
  });
});

test.describe('Tunnel: host binding', () => {
  test('a different tunnel hostname is rejected', async ({ request }) => {
    // Only the *live* tunnel host is accepted. Any other name — including
    // another trycloudflare.com subdomain — must be refused, otherwise a
    // rebound DNS name could reach kmd. See AppState::tunnel_host.
    const res = await request.get(`${ORIGIN}/api/docs`, {
      headers: { Host: 'someone-else.trycloudflare.com' },
    });
    expect(res.status()).toBe(403);
  });

  test('the live tunnel hostname is accepted', async ({ request }) => {
    const res = await request.get(`${ORIGIN}/api/docs`, { headers: AS_VISITOR });
    expect(res.status()).toBe(200);
  });
});
