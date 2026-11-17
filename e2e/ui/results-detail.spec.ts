import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { completeRun } from './helpers';

test('a passing table explains why it passed and what was tested', async ({ page }) => {
  await completeRun(page, 'pass');
  await page.getByRole('button', { name: 'Details for demo_bank.table_0000' }).click();
  const detail = page.getByTestId('table-detail');
  await expect(detail.getByText('Why this table passed')).toBeVisible();
  await expect(detail.getByTestId('why-this-table')).toContainText('Both databases hold 1,000 rows');
  await expect(detail.getByRole('region', { name: 'Columns compared and the rule for each' })).toBeVisible();
  await expect(detail.getByRole('region', { name: 'Control totals on both sides' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'What was checked' })).toBeVisible();
});

test('a failure card opens the same detail', async ({ page }) => {
  await completeRun(page, 'fail');
  await page.getByRole('button', { name: 'Show details for demo_bank.table_0001' }).click();
  await expect(page.getByText('Why this table failed')).toBeVisible();
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`results detail is accessible in ${colorScheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await completeRun(page, 'pass');
    await page.getByRole('button', { name: 'Details for demo_bank.table_0000' }).click();
    await expect(page.getByTestId('why-this-table')).toBeVisible();
    await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'));
    const results = await new AxeBuilder({ page }).analyze();
    const serious = results.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
    expect(
      serious,
      serious.map((v) => `${v.id}: ${v.nodes.map((n) => n.failureSummary).join(' | ')}`).join('\n'),
    ).toEqual([]);
  });
}
