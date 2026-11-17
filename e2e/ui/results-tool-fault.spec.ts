import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a tool fault is reported as a fault, not a data result', async ({ page }) => {
  await completeRun(page, 'toolFault');
  await expect(page.getByText('TOOL FAULT', { exact: true }).first()).toBeVisible();
  await expect(page.getByText('Failures')).toBeVisible();
  await expect(page.getByText(/spot check disagreed with the fingerprint/)).toBeVisible();
  // TOOL FAULT is its own verdict, never a plain data Fail.
  await expect(page.getByText('FAIL', { exact: true })).toHaveCount(0);
});
