import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a widely-different run reports the stop rule', async ({ page }) => {
  await completeRun(page, 'widelyDifferent');
  await expect(page.getByText('Failures')).toBeVisible();
  await expect(page.getByText(/More than 10% of buckets differ/)).toBeVisible();
  await expect(page.getByText(/drill-down was skipped/)).toBeVisible();
});
