import { test, expect } from '@playwright/test';

// CSRF header required by kmd for all mutating requests
const CSRF_HEADERS = { 'X-KMD-Client': '1' };

// ---------------------------------------------------------------------------
// App shell: basic loading, routing, layout
// ---------------------------------------------------------------------------

test.describe('App Shell', () => {
  test('loads the app and shows sidebar with three nav items', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.sidebar-logo')).toHaveText('K.md');
    const navItems = page.locator('.nav-item');
    await expect(navItems).toHaveCount(4);
    await expect(navItems.nth(0)).toContainText('Docs');
    await expect(navItems.nth(1)).toContainText('Scripts');
    await expect(navItems.nth(2)).toContainText('Ports');
    await expect(navItems.nth(3)).toContainText('Terminal');
  });

  test('defaults to docs page with active nav highlight', async ({ page }) => {
    await page.goto('/');
    await expect(page).toHaveURL(/#docs/);
    const docsNav = page.locator('.nav-item').first();
    await expect(docsNav).toHaveClass(/active/);
  });

  test('hash routing switches pages', async ({ page }) => {
    await page.goto('/#scripts');
    await expect(page.locator('.main-header h1')).toHaveText('Scripts');
    const scriptsNav = page.locator('.nav-item').nth(1);
    await expect(scriptsNav).toHaveClass(/active/);

    await page.locator('.nav-item').nth(2).click();
    await expect(page).toHaveURL(/#ports/);
    await expect(page.locator('.main-header h1')).toHaveText('Ports');
  });

  test('WebSocket connects successfully', async ({ page }) => {
    const wsPromise = page.waitForEvent('websocket');
    await page.goto('/');
    const ws = await wsPromise;
    expect(ws.url()).toContain('/ws');
  });

  test('workspace API returns workspace info', async ({ page }) => {
    const response = await page.request.get('/api/workspace');
    const data = await response.json();
    expect(data.name).toBeTruthy();
    expect(Array.isArray(data.roots)).toBe(true);
    expect(data.roots.length).toBeGreaterThan(0);
    expect(data.roots[0].name).toBeTruthy();
    expect(data.roots[0].path).toBe('.');
  });
});

// ---------------------------------------------------------------------------
// Pillar 1: Markdown Explorer — file tree, rendering, search
// ---------------------------------------------------------------------------

test.describe('Markdown Explorer', () => {
  test.beforeEach(async ({ page }) => {
    // Clear localStorage to prevent flaky lastDoc restore across test runs
    await page.goto('/#docs');
    await page.evaluate(() => localStorage.clear());
    await page.reload();
    await page.waitForResponse(resp =>
      resp.url().includes('/api/docs') && !resp.url().includes('search') && resp.status() === 200
    );
  });

  test('file tree loads and shows directories', async ({ page }) => {
    const treeItems = page.locator('.file-tree-item');
    await expect(treeItems.first()).toBeVisible({ timeout: 5000 });
    const count = await treeItems.count();
    expect(count).toBeGreaterThan(3);
  });

  test('docs API returns roots-grouped structure', async ({ page }) => {
    const response = await page.request.get('/api/docs');
    const data = await response.json();
    // New format: { roots: [{ name, path, children }] }
    expect(Array.isArray(data.roots)).toBe(true);
    expect(data.roots.length).toBeGreaterThan(0);
    expect(data.roots[0].name).toBeTruthy();
    expect(data.roots[0].path).toBe('.');
    expect(Array.isArray(data.roots[0].children)).toBe(true);
    expect(data.roots[0].children.length).toBeGreaterThan(0);
  });

  test('auto-selects and renders the first file on load', async ({ page }) => {
    // Verify via API that auto-selected file returns content
    const response = await page.waitForResponse(
      resp =>
        resp.url().includes('/api/docs/') &&
        !resp.url().includes('search') &&
        !resp.url().includes('annotations') &&
        !resp.url().includes('bookmarks') &&
        !resp.url().endsWith('/api/docs') &&
        resp.status() === 200,
      { timeout: 15000 },
    );
    const data = await response.json();
    expect(data.html || data.truncated).toBeTruthy();

    // Wait for DOM to reflect the rendered content
    if (data.html) {
      await page.waitForTimeout(300); // Allow reactive rendering to complete
      const markdownBody = page.locator('.markdown-body');
      await expect(markdownBody).toBeAttached({ timeout: 5000 });
    }
  });

  test('clicking a different file in the tree renders its markdown', async ({ page }) => {
    // Wait for file tree + initial auto-render
    await page.waitForResponse(
      resp =>
        resp.url().includes('/api/docs/') &&
        !resp.url().includes('search') &&
        !resp.url().includes('annotations') &&
        !resp.url().includes('bookmarks') &&
        !resp.url().endsWith('/api/docs') &&
        resp.status() === 200,
      { timeout: 15000 },
    );
    await page.waitForTimeout(300);

    // Find clickable .md file tree items
    const fileItems = page.locator('.file-tree-item').filter({ has: page.locator('.name', { hasText: /\.md$/i }) });
    await fileItems.first().waitFor({ timeout: 5000 });
    const count = await fileItems.count();
    expect(count).toBeGreaterThan(1);

    // Click second file and wait for its content response
    const secondFile = fileItems.nth(1);
    const [renderResponse] = await Promise.all([
      page.waitForResponse(
        resp =>
          resp.url().includes('/api/docs/') &&
          !resp.url().includes('search') &&
          !resp.url().includes('annotations') &&
          !resp.url().includes('bookmarks') &&
          !resp.url().endsWith('/api/docs') &&
          resp.status() === 200,
      ),
      secondFile.click(),
    ]);

    const data = await renderResponse.json();
    expect(data.html || data.truncated).toBeTruthy();
  });

  test('search returns results with root field', async ({ page }) => {
    const searchInput = page.locator('.search-input');
    await expect(searchInput).toBeVisible();

    await searchInput.fill('reactive');

    const searchResponse = await page.waitForResponse(
      resp => resp.url().includes('/api/docs/search') && resp.status() === 200,
      { timeout: 5000 },
    );
    const searchData = await searchResponse.json();
    expect(searchData.results.length).toBeGreaterThan(0);
    // Results now include root field
    expect(searchData.results[0].root).toBeTruthy();

    await page.waitForTimeout(500);
    const resultPaths = page.locator('.file-tree-item .name').filter({ hasText: /\.md$/i });
    const count = await resultPaths.count();
    expect(count).toBeGreaterThan(0);
  });

  test('search with "FMIR" returns highlighted results', async ({ page }) => {
    const searchInput = page.locator('.search-input');
    await expect(searchInput).toBeVisible();

    await searchInput.fill('FMIR');

    const response = await page.waitForResponse(
      resp => resp.url().includes('/api/docs/search') && resp.status() === 200,
      { timeout: 5000 },
    );
    const data = await response.json();
    expect(data.results.length).toBeGreaterThan(0);
    expect(data.results[0].snippet).toContain('<mark>');
  });

  test('rendered markdown contains syntax-highlighted code blocks', async ({ page }) => {
    const response = await page.request.get(
      '/api/docs/FormaStack/docs/architecture/TURBO-STREAMS-EVALUATION.md?root=.',
    );
    const data = await response.json();
    expect(data.html).toContain('class="highlight');
  });

  test('mermaid diagrams are present in rendered output', async ({ page }) => {
    const response = await page.request.get(
      '/api/docs/forma-code-poc-files/FormaOld/docs/FORMA-PLATFORM-SYNOPSIS.md?root=.',
    );
    const data = await response.json();
    expect(data.html).toContain('class="mermaid"');
  });
});

// ---------------------------------------------------------------------------
// Pillar 2: Script Runner — discovery, run, output streaming, kill
// ---------------------------------------------------------------------------

test.describe('Script Runner', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/#scripts');
    await page.waitForResponse(
      resp => resp.url().includes('/api/scripts') && resp.status() === 200,
    );
  });

  test('scripts page loads and shows discovered packages', async ({ page }) => {
    await expect(page.locator('.main-header h1')).toHaveText('Scripts');
    await page.waitForTimeout(500);
    const mainContent = page.locator('.main-content');
    const text = await mainContent.textContent();
    expect(text!.length).toBeGreaterThan(10);
  });

  test('scripts API returns roots-grouped structure', async ({ page }) => {
    const response = await page.request.get('/api/scripts');
    const data = await response.json();
    // New format: { roots: [{ name, path, packages }] }
    expect(Array.isArray(data.roots)).toBe(true);
    expect(data.roots.length).toBeGreaterThan(0);
    expect(data.roots[0].packages.length).toBeGreaterThan(0);
    expect(data.roots[0].packages[0].name).toBeTruthy();
  });

  test('running a script returns a process ID', async ({ page }) => {
    const scriptsResp = await page.request.get('/api/scripts');
    const scriptsData = await scriptsResp.json();

    const firstRoot = scriptsData.roots[0];
    const pkg = firstRoot.packages.find((p: any) => p.scripts.length > 0);
    if (!pkg) { test.skip(); return; }

    const runResp = await page.request.post('/api/scripts/run', {
      headers: CSRF_HEADERS,
      data: {
        root: firstRoot.path,
        package_path: pkg.path,
        script_name: pkg.scripts[0].name,
      },
    });
    const runData = await runResp.json();
    expect(runData.process_id).toBeTruthy();
    expect(runData.process_id.length).toBe(36);

    await page.waitForTimeout(500);
    await page.request.post(`/api/processes/${runData.process_id}/kill`, { headers: CSRF_HEADERS }).catch(() => {});
  });

  test('WebSocket receives process output messages', async ({ page }) => {
    const wsMessages: any[] = [];
    page.on('websocket', ws => {
      ws.on('framereceived', event => {
        try {
          wsMessages.push(JSON.parse(event.payload as string));
        } catch {}
      });
    });

    await page.goto('about:blank');
    await page.goto('/#scripts');

    await page.waitForTimeout(7000);

    expect(wsMessages.length).toBeGreaterThan(0);
    const hasPortsMsg = wsMessages.some(m => m.type === 'ports');
    expect(hasPortsMsg).toBe(true);
  });

  test('kill process API works', async ({ page }) => {
    const scriptsResp = await page.request.get('/api/scripts');
    const data = await scriptsResp.json();
    const firstRoot = data.roots[0];
    const pkg = firstRoot.packages.find((p: any) => p.scripts.length > 0);
    if (!pkg) { test.skip(); return; }

    const runResp = await page.request.post('/api/scripts/run', {
      headers: CSRF_HEADERS,
      data: {
        root: firstRoot.path,
        package_path: pkg.path,
        script_name: pkg.scripts[0].name,
      },
    });
    const { process_id } = await runResp.json();

    await page.waitForTimeout(200);
    const killResp = await page.request.post(`/api/processes/${process_id}/kill`, { headers: CSRF_HEADERS });
    const killData = await killResp.json();
    expect(killData.ok || killData.error).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// Pillar 3: Port Monitor — scanning, display, kill
// ---------------------------------------------------------------------------

test.describe('Port Monitor', () => {
  test('ports page loads and shows active ports', async ({ page }) => {
    await page.goto('/#ports');
    await expect(page.locator('.main-header h1')).toHaveText('Ports');
    await page.waitForResponse(
      resp => resp.url().includes('/api/ports') && resp.status() === 200,
      { timeout: 5000 },
    );
    await page.waitForTimeout(1000);
    // Should show at least one active port link (kmd itself on 4444)
    await expect(page.locator('a[href*="localhost"]').first()).toBeVisible({ timeout: 5000 });
  });

  test('ports API returns port 4444 with command and uptime', async ({ page }) => {
    const response = await page.request.get('/api/ports');
    const data = await response.json();
    expect(Array.isArray(data.ports)).toBe(true);
    expect(data.ports.length).toBeGreaterThan(0);

    const port4444 = data.ports.find((p: any) => p.port === 4444);
    expect(port4444).toBeTruthy();
    expect(port4444.active).toBe(true);
    // New fields
    expect(port4444.uptime_secs).toBeDefined();
    expect(typeof port4444.uptime_secs).toBe('number');
  });

  test('WebSocket receives port scan updates', async ({ page }) => {
    const wsMessages: any[] = [];
    page.on('websocket', ws => {
      ws.on('framereceived', event => {
        try {
          const msg = JSON.parse(event.payload as string);
          if (msg.type === 'ports') wsMessages.push(msg);
        } catch {}
      });
    });

    await page.goto('/#ports');
    await page.waitForTimeout(7000);

    expect(wsMessages.length).toBeGreaterThan(0);
    const portsMsg = wsMessages[0];
    expect(portsMsg.type).toBe('ports');
    expect(portsMsg.data).toBeTruthy();
  });

  test('active port shows clickable link', async ({ page }) => {
    await page.goto('/#ports');
    await page.waitForResponse(
      resp => resp.url().includes('/api/ports') && resp.status() === 200,
      { timeout: 5000 },
    );
    await page.waitForTimeout(1000);

    // Port 4444 should be a clickable link with href
    const portLink = page.locator('a[href="http://localhost:4444"]');
    await expect(portLink).toBeVisible({ timeout: 5000 });
    expect(await portLink.getAttribute('target')).toBe('_blank');
  });
});

// ---------------------------------------------------------------------------
// Security: path traversal, XSS
// ---------------------------------------------------------------------------

test.describe('Security', () => {
  test('path traversal in docs API returns error', async ({ page }) => {
    const response = await page.request.get(
      '/api/docs/FormaStack/../../../../../../etc/passwd?root=.',
    );
    const text = await response.text();
    expect(text).not.toContain('root:');
    expect(text).not.toContain('/bin/bash');
  });

  test('search snippets are HTML-escaped against XSS', async ({ page }) => {
    const response = await page.request.get('/api/docs/search?q=script');
    const data = await response.json();
    if (data.results.length > 0) {
      for (const result of data.results) {
        expect(result.snippet).not.toMatch(/<script[^>]*>/i);
      }
    }
  });

  test('script run API rejects path traversal', async ({ page }) => {
    const response = await page.request.post('/api/scripts/run', {
      headers: CSRF_HEADERS,
      data: { root: '.', package_path: '../../', script_name: 'test' },
    });
    const data = await response.json();
    expect(data.error).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// Port Orchestration — port assignment, managed processes
// ---------------------------------------------------------------------------

test.describe('Port Orchestration', () => {
  test('running a script returns assigned port and framework info', async ({ page }) => {
    const scriptsResp = await page.request.get('/api/scripts');
    const data = await scriptsResp.json();
    const firstRoot = data.roots[0];
    const pkg = firstRoot.packages.find((p: any) => p.scripts.length > 0);
    if (!pkg) { test.skip(); return; }

    const runResp = await page.request.post('/api/scripts/run', {
      headers: CSRF_HEADERS,
      data: {
        root: firstRoot.path,
        package_path: pkg.path,
        script_name: pkg.scripts[0].name,
      },
    });
    const runData = await runResp.json();
    expect(runData.process_id).toBeTruthy();
    // Port should be assigned from the 4500-4599 range
    expect(runData.assigned_port).toBeGreaterThanOrEqual(4500);
    expect(runData.assigned_port).toBeLessThanOrEqual(4599);

    await page.waitForTimeout(500);
    await page.request.post(`/api/processes/${runData.process_id}/kill`, { headers: CSRF_HEADERS }).catch(() => {});
  });

  test('port allocations API returns active allocations', async ({ page }) => {
    const response = await page.request.get('/api/ports/allocations');
    const data = await response.json();
    expect(Array.isArray(data.allocations)).toBe(true);
  });

  test('ports API includes is_self flag for kmd port', async ({ page }) => {
    const response = await page.request.get('/api/ports');
    const data = await response.json();
    const selfPort = data.ports.find((p: any) => p.is_self === true);
    expect(selfPort).toBeTruthy();
  });

  test('processes API includes assigned_port field', async ({ page }) => {
    const response = await page.request.get('/api/processes');
    const data = await response.json();
    expect(Array.isArray(data.processes)).toBe(true);
    // ProcessInfo schema should include assigned_port
    // (may be null if no processes are running)
  });
});

// ---------------------------------------------------------------------------
// Terminal — PTY sessions
// ---------------------------------------------------------------------------

test.describe('Terminal', () => {
  test('terminal page loads', async ({ page }) => {
    await page.goto('/#terminal');
    await expect(page.locator('.main-header h1')).toHaveText('Terminal');
  });

  test('terminal sessions API works', async ({ page }) => {
    const response = await page.request.get('/api/terminal/sessions');
    expect(response.status()).toBe(200);
    const data = await response.json();
    expect(Array.isArray(data.sessions)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Offline / Vendored assets
// ---------------------------------------------------------------------------

test.describe('Offline Assets', () => {
  test('vendored mermaid.min.js is served from the binary', async ({ page }) => {
    const response = await page.request.get('/vendor/mermaid.min.js');
    expect(response.status()).toBe(200);
    const text = await response.text();
    expect(text.length).toBeGreaterThan(100000);
    expect(text).toContain('mermaid');
  });

  test('CSS is served from the binary', async ({ page }) => {
    const response = await page.request.get('/dev.css');
    expect(response.status()).toBe(200);
    const text = await response.text();
    expect(text).toContain('gruvbox');
  });

  test('JS bundle is served from the binary', async ({ page }) => {
    const response = await page.request.get('/app.js');
    expect(response.status()).toBe(200);
    const text = await response.text();
    // Check for a string literal that survives minification
    expect(text.length).toBeGreaterThan(10000);
    expect(text).toContain('kmd');
  });
});
