import { test, expect } from '@playwright/test';
import { completeRun } from './helpers';

test('a failing run lists the failure cards', async ({ page }) => {
  await completeRun(page, 'fail');
  await expect(page.getByText('FAIL', { exact: true }).first()).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Failures' })).toBeVisible();
  await expect(page.getByText(/fewer rows than the old one/)).toBeVisible();
});
