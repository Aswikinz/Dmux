import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { completeRun } from './helpers';

test('the ignored list opens on request and the verdict filter hides only what is asked', async ({ page }) => {
  await completeRun(page, 'fail');
  // Ignored: counted, listed only on request.
  await expect(page.getByText('3 items, each with its reason')).toBeVisible();
  await expect(page.getByText('archived_balances')).toHaveCount(0);
  await page.getByRole('button', { name: 'Show ignored items' }).click();
  await expect(page.getByText('archived_balances')).toBeVisible();

  // Hiding PASS keeps the failures and the verdict, and says what is hidden.
  const filter = page.getByTestId('verdict-filter');
  await filter.getByRole('checkbox', { name: /PASS/ }).uncheck();
  await expect(filter.getByText(/hidden by this filter/)).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Failures' })).toBeVisible();
  await expect(page.getByText(/fewer rows than the old one/)).toBeVisible();
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`results with the filter and ignored list open are accessible in ${colorScheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await completeRun(page, 'fail');
    await page.getByRole('button', { name: 'Show ignored items' }).click();
    await page.getByTestId('verdict-filter').getByRole('checkbox', { name: /PASS/ }).uncheck();
    await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'));
    const results = await new AxeBuilder({ page }).analyze();
    const serious = results.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
    expect(
      serious,
      serious.map((v) => `${v.id}: ${v.nodes.map((n) => n.failureSummary).join(' | ')}`).join('\n'),
    ).toEqual([]);
  });
}
