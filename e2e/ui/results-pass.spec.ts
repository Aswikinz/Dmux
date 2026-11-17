import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a passing run shows the PASS verdict and coverage line', async ({ page }) => {
  await completeRun(page, 'pass');
  await expect(page.getByText('PASS', { exact: true }).first()).toBeVisible();
  await expect(page.getByText(/tables fully checked/)).toBeVisible();
  // No failures panel on a clean pass.
  await expect(page.getByRole('heading', { name: 'Failures' })).toHaveCount(0);
});
