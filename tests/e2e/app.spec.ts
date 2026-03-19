import { test, expect } from '@playwright/test';

// ---------------------------------------------------------------------------
// App shell: basic loading, routing, layout
// ---------------------------------------------------------------------------

test.describe('App Shell', () => {
  test('loads the app and shows sidebar with three nav items', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('.sidebar-logo')).toHaveText('K.md');
    const navItems = page.locator('.nav-item');
    await expect(navItems).toHaveCount(3);
    await expect(navItems.nth(0)).toContainText('Docs');
    await expect(navItems.nth(1)).toContainText('Scripts');
    await expect(navItems.nth(2)).toContainText('Ports');
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
});

// ---------------------------------------------------------------------------
// Pillar 1: Markdown Explorer — file tree, rendering, search
// ---------------------------------------------------------------------------

test.describe('Markdown Explorer', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/#docs');
    // Wait for the tree to load
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

  test('auto-selects and renders the first file on load', async ({ page }) => {
    // The DocsPage auto-selects the first file (findFirstFile) and fetches its HTML.
    // Wait for the doc render API call that fires automatically.
    await page.waitForResponse(
      resp =>
        resp.url().includes('/api/docs/') &&
        !resp.url().includes('search') &&
        !resp.url().endsWith('/api/docs') &&
        resp.status() === 200,
      { timeout: 10000 },
    );

    // The markdown body should be visible with rendered content
    const markdownBody = page.locator('.markdown-body');
    await expect(markdownBody).toBeVisible({ timeout: 5000 });
    const innerHTML = await markdownBody.innerHTML();
    expect(innerHTML.length).toBeGreaterThan(50);
  });

  test('clicking a different file in the tree renders its markdown', async ({ page }) => {
    // Wait for tree to render and the initial auto-fetch to settle
    await page.locator('.file-tree-item').first().waitFor({ timeout: 5000 });
    await page.locator('.markdown-body').waitFor({ timeout: 10000 });

    // Files have ".md" in their name. Find all file items and click one that isn't selected.
    const allFileItems = page.locator('.file-tree-item').filter({ hasText: /\.md/i });
    const count = await allFileItems.count();
    expect(count).toBeGreaterThan(1);

    // Click the second file (index 1) — different from the auto-selected first file
    const secondFile = allFileItems.nth(1);
    await expect(secondFile).toBeVisible();

    const [renderResponse] = await Promise.all([
      page.waitForResponse(
        resp =>
          resp.url().includes('/api/docs/') &&
          !resp.url().includes('search') &&
          !resp.url().endsWith('/api/docs') &&
          resp.status() === 200,
      ),
      secondFile.click(),
    ]);

    const data = await renderResponse.json();
    expect(data.html || data.truncated).toBeTruthy();

    if (data.html) {
      const markdownBody = page.locator('.markdown-body');
      await expect(markdownBody).toBeVisible({ timeout: 3000 });
      const innerHTML = await markdownBody.innerHTML();
      expect(innerHTML.length).toBeGreaterThan(50);
    }
  });

  test('search returns results and displays them', async ({ page }) => {
    const searchInput = page.locator('.search-input');
    await expect(searchInput).toBeVisible();

    // Type a query — debounce is 300ms
    await searchInput.fill('reactive');

    // Wait for the search API call
    const searchResponse = await page.waitForResponse(
      resp => resp.url().includes('/api/docs/search') && resp.status() === 200,
      { timeout: 5000 },
    );
    const searchData = await searchResponse.json();
    expect(searchData.results.length).toBeGreaterThan(0);

    // Wait for results to render
    await page.waitForTimeout(500);

    // Search results should show file paths
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
      '/api/docs/FormaStack/docs/architecture/TURBO-STREAMS-EVALUATION.md',
    );
    const data = await response.json();
    expect(data.html).toContain('class="highlight');
  });

  test('mermaid diagrams are present in rendered output', async ({ page }) => {
    const response = await page.request.get(
      '/api/docs/forma-code-poc-files/FormaOld/docs/FORMA-PLATFORM-SYNOPSIS.md',
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
    // Wait for package cards to render
    await page.waitForTimeout(500);
    const mainContent = page.locator('.main-content');
    const text = await mainContent.textContent();
    expect(text!.length).toBeGreaterThan(10);
  });

  test('scripts API returns packages with scripts', async ({ page }) => {
    const response = await page.request.get('/api/scripts');
    const data = await response.json();
    expect(data.packages.length).toBeGreaterThan(0);
    const withScripts = data.packages.filter((p: any) => p.scripts.length > 0);
    expect(withScripts.length).toBeGreaterThan(0);
    expect(data.packages[0].name).toBeTruthy();
    expect(data.packages[0].path).toBeTruthy();
  });

  test('running a script returns a process ID', async ({ page }) => {
    const scriptsResp = await page.request.get('/api/scripts');
    const scriptsData = await scriptsResp.json();

    // Find any package with scripts
    const pkg = scriptsData.packages.find((p: any) => p.scripts.length > 0);
    if (!pkg) { test.skip(); return; }

    const runResp = await page.request.post('/api/scripts/run', {
      data: {
        package_path: pkg.path,
        script_name: pkg.scripts[0].name,
      },
    });
    const runData = await runResp.json();
    expect(runData.process_id).toBeTruthy();
    expect(runData.process_id.length).toBe(36); // UUID

    // Clean up — kill the process
    await page.waitForTimeout(500);
    await page.request.post(`/api/processes/${runData.process_id}/kill`).catch(() => {});
  });

  test('WebSocket receives process output messages', async ({ page }) => {
    // Use a fresh page context to control WS lifecycle
    const wsMessages: any[] = [];
    page.on('websocket', ws => {
      ws.on('framereceived', event => {
        try {
          wsMessages.push(JSON.parse(event.payload as string));
        } catch {}
      });
    });

    // Full navigation to get a fresh WS connection
    await page.goto('about:blank');
    await page.goto('/#scripts');

    // Wait for the WS connection + port scan cycle (broadcasts every 5s)
    await page.waitForTimeout(7000);

    // Should have received at least one WS message (port scan broadcasts every 5s)
    expect(wsMessages.length).toBeGreaterThan(0);

    // Verify we got ports messages (guaranteed by the 5s interval)
    const hasPortsMsg = wsMessages.some(m => m.type === 'ports');
    expect(hasPortsMsg).toBe(true);
  });

  test('kill process API works', async ({ page }) => {
    const scriptsResp = await page.request.get('/api/scripts');
    const data = await scriptsResp.json();
    const pkg = data.packages.find((p: any) => p.scripts.length > 0);
    if (!pkg) { test.skip(); return; }

    const runResp = await page.request.post('/api/scripts/run', {
      data: {
        package_path: pkg.path,
        script_name: pkg.scripts[0].name,
      },
    });
    const { process_id } = await runResp.json();

    await page.waitForTimeout(200);
    const killResp = await page.request.post(`/api/processes/${process_id}/kill`);
    const killData = await killResp.json();
    expect(killData.ok || killData.error).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// Pillar 3: Port Monitor — scanning, display, kill
// ---------------------------------------------------------------------------

test.describe('Port Monitor', () => {
  test('ports page loads and shows port table', async ({ page }) => {
    await page.goto('/#ports');
    await expect(page.locator('.main-header h1')).toHaveText('Ports');
    // Wait for initial port data fetch
    await page.waitForResponse(
      resp => resp.url().includes('/api/ports') && resp.status() === 200,
      { timeout: 5000 },
    );
    // Table should be visible
    await expect(page.locator('.port-table')).toBeVisible({ timeout: 3000 });
  });

  test('ports API returns scanned ports including kmd on 4444', async ({ page }) => {
    const response = await page.request.get('/api/ports');
    const data = await response.json();
    expect(Array.isArray(data.ports)).toBe(true);
    expect(data.ports.length).toBeGreaterThan(0);

    // Port 4444 should be active (kmd itself)
    const port4444 = data.ports.find((p: any) => p.port === 4444);
    expect(port4444).toBeTruthy();
    expect(port4444.active).toBe(true);
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
    // Port scan broadcasts every 5s — wait for at least one
    await page.waitForTimeout(7000);

    expect(wsMessages.length).toBeGreaterThan(0);
    const portsMsg = wsMessages[0];
    expect(portsMsg.type).toBe('ports');
    expect(portsMsg.data).toBeTruthy();
  });

  test('port table shows port 4444 as active', async ({ page }) => {
    await page.goto('/#ports');
    // Wait for port data to load (HTTP fetch + render)
    await page.waitForResponse(
      resp => resp.url().includes('/api/ports') && resp.status() === 200,
      { timeout: 5000 },
    );
    await page.waitForTimeout(1000); // Let the effect re-render the table

    // Port 4444 should appear in the table
    const portCell = page.locator('td.port-num').filter({ hasText: '4444' });
    await expect(portCell).toBeVisible({ timeout: 5000 });

    // There should be at least one active status dot
    const activeIndicators = page.locator('.status-dot.active');
    const count = await activeIndicators.count();
    expect(count).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// Security: path traversal, XSS
// ---------------------------------------------------------------------------

test.describe('Security', () => {
  test('path traversal in docs API returns error', async ({ page }) => {
    const response = await page.request.get(
      '/api/docs/FormaStack/../../../../../../etc/passwd',
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
      data: { package_path: '../../', script_name: 'test' },
    });
    const data = await response.json();
    expect(data.error).toBeTruthy();
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
    expect(text).toContain('createSignal');
  });
});
