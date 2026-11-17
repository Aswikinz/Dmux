import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a widely-different run reports its own verdict and the stop rule', async ({ page }) => {
  await completeRun(page, 'widelyDifferent');
  await expect(page.getByText('WIDELY DIFFERENT', { exact: true }).first()).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Failures' })).toBeVisible();
  await expect(page.getByText(/More than 10% of buckets differ/)).toBeVisible();
  await expect(page.getByText(/drill-down was skipped/)).toBeVisible();
  // WIDELY DIFFERENT is its own verdict, not folded into a plain Fail.
  await expect(page.getByText('FAIL', { exact: true })).toHaveCount(0);
});
