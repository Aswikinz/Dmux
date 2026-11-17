import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a changed-during-check run is never a PASS', async ({ page }) => {
  await completeRun(page, 'changedDuringCheck');
  await expect(page.getByText('CHANGED DURING CHECK', { exact: true }).first()).toBeVisible();
  await expect(page.getByText(/counters moved during the check/)).toBeVisible();
});
