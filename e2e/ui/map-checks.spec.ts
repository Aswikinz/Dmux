import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { fillProfiles } from './helpers';

const cta = (page: Page, name: string) =>
  page.getByRole('main').getByRole('button', { name, exact: true });

/** Connect with a differing old database and open the Map screen. */
async function openMap(page: Page) {
  await page.goto('/');
  await fillProfiles(page, { oldDatabase: 'diff' });
  const tests = page.getByRole('button', { name: 'Test connection' });
  await tests.nth(0).click();
  await tests.nth(1).click();
  await expect(page.getByText('Server version:').nth(1)).toBeVisible();
  await cta(page, 'Compare structure').click();
  await cta(page, 'Map tables').click();
  await expect(page.getByText(/items need attention/)).toBeVisible();
}

async function settle(page: Page) {
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running'));
}

async function noSeriousViolations(page: Page, where: string) {
  await settle(page);
  const results = await new AxeBuilder({ page }).analyze();
  const serious = results.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
  expect(
    serious,
    `${where}: ${serious.map((v) => `${v.id}: ${v.nodes.map((n) => n.failureSummary).join(' | ')}`).join('\n')}`,
  ).toEqual([]);
}

test('a suggestion is shown with its confidence and used only when accepted', async ({ page }) => {
  await openMap(page);
  await page.getByText('demo_bank.legacy_ledger').click();
  await expect(page.getByTestId('suggestion-line').first()).toContainText('82% match');
  await page.getByRole('button', { name: 'Use suggestion' }).click();
  await expect(page.getByText('demo_bank.legacy_ledger')).toHaveCount(0);

  // A second old table for the same new table is refused, in plain words.
  await page.getByText('demo_bank.ledger_old').click();
  await page.getByRole('button', { name: 'Use suggestion' }).click();
  await expect(page.getByText(/Two old tables cannot map to one new table/)).toBeVisible();
});

test('the pairing picker works with the keyboard alone', async ({ page }) => {
  await openMap(page);
  await page.getByText('demo_bank.audit_trail').click();
  await page.getByRole('button', { name: 'Choose counterpart' }).focus();
  await page.keyboard.press('Enter');
  const box = page.getByRole('combobox', { name: /Counterpart for demo_bank.audit_trail/ });
  await expect(box).toBeFocused();
  await page.keyboard.type('audit');
  await expect(page.getByRole('option')).toHaveCount(1);
  await page.keyboard.press('Enter');
  await expect(page.getByText('demo_bank.audit_trail')).toHaveCount(0);
});

for (const colorScheme of ['light', 'dark'] as const) {
  test(`the rule editor, picker and selection tools are accessible in ${colorScheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await openMap(page);
    await page.getByRole('button', { name: /Add schema rename/ }).click();
    await page.getByRole('button', { name: /Add prefix or suffix rule/ }).click();
    await page.getByLabel('Rule 1: old schema').fill('legacy');
    await page.getByLabel('Rule 1: new schema').fill('demo_bank');
    await page.getByLabel('Rule 2: old text').fill('tbl_');
    await page.getByRole('button', { name: 'Preview rules' }).click();
    await expect(page.getByText(/These rules pair/)).toBeVisible();
    await page.getByText('demo_bank.audit_trail').click();
    await page.getByRole('button', { name: 'Choose counterpart' }).click();
    await noSeriousViolations(page, 'map with picker and rules');

    await page.keyboard.press('Escape');
    await cta(page, 'Choose checks').click();
    // Nothing is chosen until the user picks the tables to check.
    await page.getByRole('button', { name: 'Select all', exact: true }).click();
    await expect(page.getByText('Estimated time:')).toBeVisible();
    await page.getByText('Change several tables at once').click();
    await page.getByLabel('Schema', { exact: true }).selectOption('archive');
    await page.getByRole('button', { name: 'Select schema' }).click();
    await expect(page.getByText(/tables? selected \(schema archive\)/)).toBeVisible();
    await noSeriousViolations(page, 'choose checks with a selection');
  });
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`what was mapped can be shown and is accessible in ${colorScheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await openMap(page);
    await page.getByRole('button', { name: 'Show what was mapped' }).click();
    const list = page.getByTestId('matched-list');
    await expect(list).toBeVisible();
    await list.getByRole('button', { name: 'Show the columns of accounts' }).click();
    await expect(list.getByText('Needs your decision').first()).toBeVisible();
    await noSeriousViolations(page, 'map with what was mapped open');
    // In the browser demo, Save mapping says so instead of failing silently.
    await page.getByRole('button', { name: 'Save mapping' }).click();
    await expect(page.getByText(/This is the browser demo, so no file was saved/)).toBeVisible();
  });
}

test('bulk set level marks the preset Custom, and control totals start unticked', async ({ page }) => {
  await openMap(page);
  await cta(page, 'Choose checks').click();
  // Nothing is chosen until the user picks the tables to check.
  await page.getByRole('button', { name: 'Select all', exact: true }).click();
  await expect(page.getByText('Estimated time:')).toBeVisible();
  // The bulk tools stay folded away until asked for.
  await expect(page.getByLabel('Name pattern')).toBeHidden();
  await page.getByText('Change several tables at once').click();
  await page.getByLabel('Name pattern').fill('table_000*');
  await page.getByRole('button', { name: 'Select by name' }).click();
  await page.getByLabel('Set level for the selection').selectOption('structure');
  await page.getByRole('button', { name: /^Apply to \d+ selected$/ }).click();
  await expect(page.getByTestId('preset-select')).toHaveValue('custom');
  await expect(page.getByTestId('totals-helper')).toContainText('nothing is added up yet');
  // A table's columns open in place: keys locked in, totals per table.
  await page.getByRole('button', { name: /^Choose columns of / }).first().click();
  const chooser = page.getByTestId('column-chooser');
  await expect(chooser.getByRole('heading', { name: /^Columns of / })).toBeFocused();
  await expect(chooser.getByRole('checkbox', { name: 'Compare id' })).toBeDisabled();
  await chooser.getByRole('checkbox', { name: 'Compare note' }).uncheck();
  await expect(page.getByTestId('left-out-columns')).toContainText('1 column is left out');
  await noSeriousViolations(page, 'choose checks with a column chooser open');
  await chooser.getByRole('button', { name: 'Done' }).click();
  await expect(chooser).toBeHidden();
  await page.getByRole('button', { name: 'Add up all suggested' }).click();
  await expect(page.getByTestId('totals-helper')).toContainText('added up');
  // The browser demo cannot save files, and says so.
  await page.getByRole('button', { name: 'Save these choices…' }).click();
  await expect(page.getByTestId('check-choices-message')).toContainText('browser demo cannot save files');
});
