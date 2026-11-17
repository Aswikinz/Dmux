import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a tool fault is reported as a fault, not a data result', async ({ page }) => {
  await completeRun(page, 'toolFault');
  await expect(page.getByText('Failures')).toBeVisible();
  await expect(page.getByText(/spot check disagreed with the fingerprint/)).toBeVisible();
});
