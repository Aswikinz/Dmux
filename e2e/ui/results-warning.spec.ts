import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a warning run shows the WARNING verdict', async ({ page }) => {
  await completeRun(page, 'warning');
  await expect(page.getByText('WARNING', { exact: true }).first()).toBeVisible();
  await expect(page.getByText(/ANALYZE has not run/)).toBeVisible();
});
