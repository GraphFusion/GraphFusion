import { test, expect } from '@playwright/test';

test('homepage leads to a readable quickstart and copied GQL', async ({ page, context }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const failedAssets: string[] = [];
  page.on('response', response => { if (response.status() >= 400 && /\.(css|js|svg|wasm)(\?|$)/.test(response.url())) failedAssets.push(response.url()); });
  await page.goto('/GraphFusion/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Graph queries. Columnar execution.');
  await page.getByRole('link', { name: 'Run your first graph query' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Your first graph');
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  const block = page.locator('.expressive-code').first();
  await block.hover();
  await block.getByRole('button').click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('CREATE GRAPH social ANY GRAPH;');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true);
  expect(errors).toEqual([]);
  expect(failedAssets).toEqual([]);
});

test('search finds a feature page with a correct project path', async ({ page }) => {
  await page.goto('/GraphFusion/');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.getByRole('textbox', { name: 'Search', exact: true }).fill('PERCENTILE_CONT');
  const result = page.locator('.pagefind-ui__result-link').filter({ hasText: 'PERCENTILE_CONT and PERCENTILE_DISC' }).first();
  await expect(result).toBeVisible();
  await result.click();
  await expect(page).toHaveURL(/\/GraphFusion\/expressions\/percentiles\//);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('PERCENTILE_CONT and PERCENTILE_DISC');
});

test('navigation and theme controls work on narrow screens', async ({ page, isMobile }) => {
  await page.goto('/GraphFusion/start/quickstart/');
  if (isMobile) await page.getByRole('button', { name: 'Menu', exact: true }).click();
  const sidebar = page.locator('#starlight__sidebar');
  await sidebar.getByRole('link', { name: 'Supported features', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Supported features');
  if (isMobile) await page.getByRole('button', { name: 'Menu', exact: true }).click();
  await page.getByRole('combobox', { name: 'Select theme' }).selectOption('dark');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  if (isMobile) await page.getByRole('button', { name: 'Menu', exact: true }).click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true);
});
