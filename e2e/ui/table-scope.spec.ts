import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { fillProfiles } from './helpers';

const cta = (page: Page, name: string) =>
  page.getByRole('main').getByRole('button', { name, exact: true });

/** Connect, compare and map, then open Choose checks. */
async function openChecks(page: Page) {
  await page.goto('/');
  await fillProfiles(page);
  const tests = page.getByRole('button', { name: 'Test connection' });
  await tests.nth(0).click();
  await tests.nth(1).click();
  await expect(page.getByText('Server version:').nth(1)).toBeVisible();
  await cta(page, 'Compare structure').click();
  await cta(page, 'Map tables').click();
  await cta(page, 'Choose checks').click();
  await expect(page.getByTestId('table-scope')).toBeVisible();
}

test('nothing is chosen at first; a few tables can be chosen and run on their own', async ({ page }) => {
  await openChecks(page);
  await expect(page.getByText('No tables chosen yet (40 available).')).toBeVisible();
  await expect(cta(page, 'Run')).toBeDisabled();

  // Open a schema and tick two tables: the schema box shows "some".
  await page.getByRole('button', { name: 'Show the tables in demo_bank' }).click();
  await page.getByRole('checkbox', { name: 'Check demo_bank.table_0001' }).check();
  await page.getByRole('checkbox', { name: 'Check demo_bank.table_0002' }).check();
  await expect(page.getByText('2 of 40 tables chosen. The other 38 will not be read.')).toBeVisible();
  await expect(page.getByRole('checkbox', { name: 'Every table in demo_bank' })).toHaveAttribute(
    'aria-checked',
    'mixed',
  );
  // Only the chosen tables take a level.
  await expect(page.getByRole('combobox', { name: /^Check level for / })).toHaveCount(2);

  await cta(page, 'Run').click();
  await expect(page.getByText('The run has finished.')).toBeVisible();
  // The run read only the two chosen tables.
  await expect(page.getByText('demo_bank.table_0001')).toBeVisible();
  await expect(page.getByText('demo_bank.table_0002')).toBeVisible();
  await expect(page.getByText('demo_bank.table_0003')).toHaveCount(0);

  // Results name every left-out table with the app's reason.
  await cta(page, 'See results').click();
  await page.getByRole('button', { name: 'Show ignored items' }).click();
  await expect(page.getByText(/not selected for this run/).first()).toBeVisible();
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the table chooser is accessible in ${colorScheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await openChecks(page);
    await page.getByRole('button', { name: 'Show the tables in demo_bank' }).click();
    await page.getByRole('checkbox', { name: 'Check demo_bank.table_0001' }).check();
    await page.getByPlaceholder('Find a schema or table').fill('table_00');
    await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'));
    const results = await new AxeBuilder({ page }).analyze();
    const serious = results.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
    expect(
      serious,
      serious.map((v) => `${v.id}: ${v.nodes.map((n) => n.failureSummary).join(' | ')}`).join('\n'),
    ).toEqual([]);
  });
}
