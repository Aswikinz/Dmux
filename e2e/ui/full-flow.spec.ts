import { test, expect } from '@playwright/test';
import { setFixture, connect } from './helpers';

test('completes the full 7-step flow and exports a report', async ({ page }) => {
  await setFixture(page, 'pass');
  await page.goto('/');

  // 1. Connect
  await connect(page);

  // 2. Compare structure
  await expect(page.getByText('demo_bank').first()).toBeVisible();

  // 3. Map
  await page.getByRole('main').getByRole('button', { name: 'Map tables' }).click();
  await expect(page.getByText(/No items need attention|items need attention/)).toBeVisible();

  // 4. Choose checks
  await page.getByRole('main').getByRole('button', { name: 'Choose checks' }).click();
  await expect(page.getByText('Estimate:')).toBeVisible();

  // 5. Run
  await page.getByRole('main').getByRole('button', { name: 'Run' }).click();
  await expect(page.getByText('Finished')).toBeVisible();

  // 6. Results
  await page.getByRole('main').getByRole('button', { name: 'See results' }).click();
  await expect(page.getByText(/tables fully checked/)).toBeVisible();

  // 7. Report
  await page.getByRole('main').getByRole('button', { name: 'Report' }).click();
  await expect(page.getByRole('button', { name: /Export PDF/ })).toBeVisible();
  await page.getByRole('button', { name: /Export PDF/ }).click();
  await expect(page.getByText(/report saved/)).toBeVisible();
});
