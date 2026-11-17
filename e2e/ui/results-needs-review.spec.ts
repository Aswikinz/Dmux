import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a needs-review run is never counted as a pass', async ({ page }) => {
  await completeRun(page, 'needsReview');
  await expect(page.getByText('NEEDS REVIEW', { exact: true }).first()).toBeVisible();
  await expect(page.getByText('Failures')).toBeVisible();
  await expect(page.getByText(/manual review/)).toBeVisible();
  // NEEDS REVIEW is its own verdict, never folded into a plain data Fail.
  await expect(page.getByText('FAIL', { exact: true })).toHaveCount(0);
});
