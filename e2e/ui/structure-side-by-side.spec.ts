import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { fillProfiles, setFixture } from './helpers';

async function openStructure(page: Page, database: string): Promise<void> {
  await setFixture(page, 'pass');
  await page.goto('/');
  await fillProfiles(page, { oldDatabase: database });
  const tests = page.getByRole('button', { name: 'Test connection' });
  await tests.nth(0).click();
  await expect(page.getByText('Server version:').first()).toBeVisible();
  await tests.nth(1).click();
  await expect(page.getByText('Server version:').nth(1)).toBeVisible();
  await page.getByRole('main').getByRole('button', { name: 'Compare structure', exact: true }).click();
  await expect(page.getByTestId('structure-compare')).toBeVisible();
}

const rows = (page: Page) => page.getByTestId('structure-row');

test('old is on the left, new on the right, with gaps and words for what differs', async ({ page }) => {
  await openStructure(page, 'diff');
  await expect(page.getByTestId('old-header')).toContainText('Old database');
  await expect(page.getByTestId('old-header')).toContainText('LATIN1');
  await expect(page.getByTestId('new-header')).toContainText('New database');
  await expect(page.getByTestId('new-header')).toContainText('UTF8');

  const balance = rows(page).filter({ hasText: /^balance/ });
  await expect(balance.locator('[data-side="old"]')).toHaveText('numeric(18,2)');
  await expect(balance.locator('[data-side="new"]')).toHaveText('numeric(19,2)');
  await expect(balance.getByText('Changed', { exact: true })).toBeVisible();

  const memo = rows(page).filter({ hasText: /^memo/ });
  await expect(memo.locator('[data-side="old"]')).toHaveText('Not in the old database');
  await expect(memo.getByText('Added', { exact: true })).toBeVisible();

  const archived = rows(page).filter({ hasText: 'archived_balances' });
  await expect(archived.locator('[data-side="new"]')).toHaveText('Not in the new database');
  await expect(archived.getByText('Missing', { exact: true })).toBeVisible();
});

test('search and show-only-problems narrow the list', async ({ page }) => {
  await openStructure(page, 'diff');
  await page.getByLabel('Search the structure').fill('transactions');
  await expect(rows(page).filter({ hasText: 'transactions' }).first()).toBeVisible();
  await expect(rows(page).filter({ hasText: 'accounts' })).toHaveCount(0);
  await page.getByLabel('Search the structure').fill('');
  await page.getByText('Show only problems').click();
  await expect(rows(page).filter({ hasText: 'name' })).toHaveCount(0);
  await expect(rows(page).filter({ hasText: /^balance/ })).toHaveCount(1);
});

test('the tree grid can be driven from the keyboard', async ({ page }) => {
  await openStructure(page, 'diff');
  const grid = page.getByRole('treegrid');
  await grid.focus();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowDown');
  const active = await grid.getAttribute('aria-activedescendant');
  expect(active).toMatch(/-row-2$/);
  await page.keyboard.press('End');
  expect(await grid.getAttribute('aria-activedescendant')).toMatch(/-row-\d+$/);
  await page.keyboard.press('Home');
  expect(await grid.getAttribute('aria-activedescendant')).toMatch(/-row-0$/);
});

test('two thousand tables stay smooth: only visible lines are drawn and search is quick', async ({ page }) => {
  await openStructure(page, 'scale');
  await expect(page.getByTestId('structure-count')).toContainText('of 11,');
  // Virtualised: far fewer lines in the page than items in the list.
  const drawn = await rows(page).count();
  expect(drawn).toBeGreaterThan(5);
  expect(drawn).toBeLessThan(120);

  // Scrolling to the end brings the last table into view.
  const scroller = page.getByRole('treegrid');
  await scroller.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await expect(rows(page).filter({ hasText: 'table_019' }).first()).toBeVisible();

  // Search across all 2,000 tables returns quickly.
  const start = Date.now();
  await page.getByLabel('Search the structure').fill('table_01999');
  await expect(rows(page).filter({ hasText: 'table_01999' }).first()).toBeVisible();
  expect(Date.now() - start).toBeLessThan(3000);
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`side-by-side structure is accessible in ${colorScheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await openStructure(page, 'diff');
    await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'));
    const results = await new AxeBuilder({ page }).analyze();
    const serious = results.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
    expect(
      serious,
      serious.map((v) => `${v.id}: ${v.nodes.map((n) => n.failureSummary).join(' | ')}`).join('\n'),
    ).toEqual([]);
  });
}

test('kinds of object can be hidden so table and column changes stand out', async ({ page }) => {
  await openStructure(page, 'diff');
  const kinds = page.getByTestId('structure-kinds');
  await expect(kinds.getByRole('checkbox', { name: 'Tables and columns' })).toBeChecked();
  await kinds.getByRole('button', { name: 'Tables and columns only' }).click();
  await expect(kinds.getByRole('checkbox', { name: 'Indexes and constraints' })).not.toBeChecked();
  await expect(kinds.getByRole('checkbox', { name: 'Other PostgreSQL objects' })).not.toBeChecked();
  // Table and column changes are still there.
  await expect(rows(page).filter({ hasText: /^balance/ })).toBeVisible();
  await kinds.getByRole('button', { name: 'Show every kind' }).click();
  await expect(kinds.getByRole('checkbox', { name: 'Indexes and constraints' })).toBeChecked();
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'));
  const results = await new AxeBuilder({ page }).analyze();
  const serious = results.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
  expect(serious.map((v) => v.id)).toEqual([]);
});
