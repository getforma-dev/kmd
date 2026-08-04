import { test, expect } from '@playwright/test';

/**
 * The phone layout is a distinct UI, not a narrow desktop: a compact header,
 * a bottom tab bar, and slide-over panels for the file tree and table of
 * contents. These run under the `mobile` project (Pixel 5 viewport + touch).
 */

test.beforeEach(async ({ page }) => {
  await page.goto('/#docs');
  await page.waitForTimeout(1000);
});

test('the body is flagged as mobile so the CSS tier applies', async ({ page }) => {
  await expect(page.locator('body')).toHaveClass(/mobile/);
});

test('the bottom tab bar offers all four sections to the owner', async ({ page }) => {
  const tabs = page.locator('.bottom-tab-bar .tab-item, .bottom-tab-bar button, .bottom-tab-bar a');
  await expect(tabs.first()).toBeVisible();
  expect(await tabs.count()).toBeGreaterThanOrEqual(4);
});

test('the desktop sidebar is not rendered inline on a phone', async ({ page }) => {
  // The sidebar becomes a slide-over overlay; it must not occupy layout width.
  const visible = await page.evaluate(() => {
    const el = document.querySelector('.sidebar') as HTMLElement | null;
    if (!el) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.left >= 0 && r.left < window.innerWidth / 2;
  });
  expect(visible).toBe(false);
});

test('markdown renders and is readable at phone width', async ({ page }) => {
  await expect(page.locator('.markdown-body')).not.toBeEmpty();

  // Nothing may force the page to scroll sideways.
  const overflows = await page.evaluate(
    () => document.documentElement.scrollWidth > window.innerWidth + 1,
  );
  expect(overflows).toBe(false);
});

test('tapping a bottom tab changes route', async ({ page }) => {
  const tabs = page.locator('.bottom-tab-bar .tab-item, .bottom-tab-bar button, .bottom-tab-bar a');
  await tabs.nth(1).tap();
  await page.waitForTimeout(600);
  expect(await page.evaluate(() => location.hash)).not.toBe('#docs');
});
