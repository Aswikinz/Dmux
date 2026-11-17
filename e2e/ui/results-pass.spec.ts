import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a passing run shows the PASS verdict and coverage line', async ({ page }) => {
  await completeRun(page, 'pass');
  await expect(page.getByText('Pass').first()).toBeVisible();
  await expect(page.getByText(/tables fully checked/)).toBeVisible();
  // No failures panel on a clean pass.
  await expect(page.getByText('Failures')).toHaveCount(0);
});
