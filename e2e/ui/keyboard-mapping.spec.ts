import { test, expect } from '@playwright/test';
import { fillProfiles } from './helpers';

test('maps the needs-attention queue using only the keyboard', async ({ page }) => {
  await page.goto('/');

  // Point the old database at 'diff' so the mock returns a differing inventory,
  // which produces a non-empty needs-attention queue.  The form no longer
  // pre-fills values, so fill both sides explicitly.
  await fillProfiles(page, { oldDatabase: 'diff' });

  const testButtons = page.getByRole('button', { name: 'Test connection' });
  await testButtons.nth(0).click();
  await expect(page.getByText('Server version:').first()).toBeVisible();
  await testButtons.nth(1).click();
  await expect(page.getByText('Server version:').nth(1)).toBeVisible();

  await page.getByRole('main').getByRole('button', { name: 'Compare structure' }).click();
  await expect(page.getByRole('main').getByRole('button', { name: 'Map tables' })).toBeVisible();
  await page.getByRole('main').getByRole('button', { name: 'Map tables' }).click();

  // The queue is keyboard-driven: ↑/↓ move, Enter approves, Delete ignores.
  await expect(page.getByText(/items need attention/)).toBeVisible();

  // Approve the first item.
  await page.keyboard.press('Enter');
  await expect(page.getByText(/items need attention/)).toBeVisible();

  // Ignore the next item, entering a required reason.
  await page.keyboard.press('Delete');
  const reason = page.getByLabel('Reason for ignoring');
  await expect(reason).toBeVisible();
  await reason.fill('out of scope');
  await page.keyboard.press('Enter');

  // The ignored item is recorded.
  await expect(page.getByRole('heading', { name: 'Ignored' })).toBeVisible();
});
