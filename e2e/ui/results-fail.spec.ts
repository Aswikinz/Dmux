import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a failing run lists the failure cards', async ({ page }) => {
  await completeRun(page, 'fail');
  await expect(page.getByText('Fail').first()).toBeVisible();
  await expect(page.getByText('Failures')).toBeVisible();
  await expect(page.getByText(/fewer rows than the old one/)).toBeVisible();
});
