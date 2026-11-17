import { test, expect } from '@playwright/test';
import { fillProfiles } from './helpers';

const library = (page: import('@playwright/test').Page) =>
  page.getByRole('navigation', { name: 'Library' }).getByRole('button', { name: 'History' });

test('History lists finished runs and opens one on the Results screen', async ({ page }) => {
  await page.goto('/');
  await library(page).click();
  await expect(page.getByRole('heading', { name: 'History', exact: true })).toBeVisible();
  // Each row has the verdict as a word, not only a colour.
  await expect(page.getByText('FAIL', { exact: true }).first()).toBeVisible();
  await expect(page.getByText('PASS', { exact: true }).first()).toBeVisible();

  await page.getByRole('button', { name: /Open results from 2026-11-16/ }).click();
  await expect(page.getByRole('heading', { name: 'Failures' })).toBeVisible();
  await expect(page.getByTestId('verdict-word')).toHaveText(/FAIL/);

  // The opened results can be exported from the Report screen.
  await page.getByRole('main').getByRole('button', { name: 'Report', exact: true }).click();
  await page.getByRole('button', { name: 'Export PDF' }).click();
  await expect(page.getByText(/PDF report saved to/)).toBeVisible();
});

test('Back from opened results returns to History', async ({ page }) => {
  await page.goto('/');
  await library(page).click();
  await page.getByRole('button', { name: /Open results from 2026-11-15/ }).click();
  await expect(page.getByTestId('verdict-word')).toBeVisible();
  await page.getByRole('main').getByRole('button', { name: 'Back', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'History', exact: true })).toBeVisible();
});

test('the run log shows as text and can be saved', async ({ page }) => {
  await page.goto('/');
  await library(page).click();
  await page.getByRole('button', { name: /Show the run log from 2026-11-16/ }).click();
  await expect(page.getByTestId('run-log-text')).toContainText('verdict: FAIL');
  await page.getByRole('button', { name: 'Save run log' }).click();
  await expect(page.getByText(/Run log saved to/)).toBeVisible();
});

test('opening a results file shows its results', async ({ page }) => {
  await page.goto('/');
  await library(page).click();
  await page.getByRole('button', { name: 'Open a results file' }).click();
  await expect(page.getByRole('heading', { name: 'Failures' })).toBeVisible();
});

test('deleting local history asks first and then empties the list', async ({ page }) => {
  await page.goto('/');
  await library(page).click();
  await page.getByRole('button', { name: 'Delete all history' }).click();
  await expect(page.getByText(/cannot be undone/)).toBeVisible();
  await page.getByRole('button', { name: 'Keep my history' }).click();
  await expect(page.getByText('2026-11-16 09:48 UTC')).toBeVisible();

  await page.getByRole('button', { name: 'Delete all history' }).click();
  await page.getByRole('button', { name: 'Yes, delete everything' }).click();
  await expect(page.getByText(/No finished checks yet/)).toBeVisible();
});

test('a tested connection is saved automatically and can be retried, picked and deleted', async ({ page }) => {
  await page.goto('/');
  await fillProfiles(page);
  await page.getByRole('button', { name: 'Test connection' }).first().click();
  const card = page.getByTestId('Old database-saved-card');
  await expect(card.getByText('Saved', { exact: true })).toBeVisible();
  await expect(card.getByText(/Last tested/)).toBeVisible();

  await card.getByRole('button', { name: 'Retry connection' }).click();
  await expect(card.getByText(/Last tested/)).toBeVisible();

  // It is offered on the New side's picker too (but not usable on both sides).
  await expect(page.getByTestId('New database-saved-select')).toBeEnabled();

  await card.getByRole('button', { name: 'Delete connection' }).click();
  await card.getByRole('button', { name: 'Yes, delete it' }).click();
  await expect(page.getByText(/Deleted the saved connection/)).toBeVisible();
  await expect(page.getByTestId('Old database-saved-card')).toHaveCount(0);
});
