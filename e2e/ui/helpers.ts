import { expect, type Page } from '@playwright/test';

/**
 * The app reads `window.__dmuxForcedFixture` once, when it mounts, to decide
 * which deterministic mock result set to show. Setting it before any page
 * script runs lets each test pick its own result type.
 */
export async function setFixture(page: Page, fixture: string): Promise<void> {
  await page.addInitScript((value) => {
    window.__dmuxForcedFixture = value;
  }, fixture);
}

/** Primary call-to-action buttons live inside <main>; the sidebar repeats step names. */
function cta(page: Page, name: string) {
  return page.getByRole('main').getByRole('button', { name, exact: true });
}

/**
 * Fill the two connection profiles explicitly.  The Connect form no longer
 * pre-fills mock values (empty defaults), so tests that drive the real form
 * must supply them.  `database` defaults to 'demo_bank' (the mock's base
 * inventory) but may be overridden to 'diff' etc.
 */
export async function fillProfiles(
  page: Page,
  overrides: { oldDatabase?: string; newDatabase?: string } = {},
): Promise<void> {
  const fill = async (side: 'Old database' | 'New database', database: string) => {
    await page.getByTestId(`${side}-host`).fill('127.0.0.1');
    await page.getByTestId(`${side}-port`).fill(side === 'Old database' ? '5433' : '5434');
    await page.getByTestId(`${side}-database`).fill(database);
    await page.getByTestId(`${side}-user`).fill('dmux_dev');
  };
  await fill('Old database', overrides.oldDatabase ?? 'demo_bank');
  await fill('New database', overrides.newDatabase ?? 'demo_bank');
}

/** Connect both databases on the Connect screen and move to the next step. */
export async function connect(page: Page): Promise<void> {
  await fillProfiles(page);
  const testButtons = page.getByRole('button', { name: 'Test connection' });
  await testButtons.nth(0).click();
  await expect(page.getByText('Server version:').first()).toBeVisible();
  await testButtons.nth(1).click();
  await expect(page.getByText('Server version:').nth(1)).toBeVisible();
  await cta(page, 'Compare structure').click();
  await expect(cta(page, 'Map tables')).toBeVisible();
}

/**
 * Walk the full guided flow from the Connect screen to the Results screen,
 * stopping once the coverage line is visible. Returns the results screen body
 * for further assertions.
 */
export async function completeRun(page: Page, fixture: string): Promise<void> {
  await setFixture(page, fixture);
  await page.goto('/');
  await connect(page);

  // Compare structure -> Map
  await cta(page, 'Map tables').click();
  await expect(page.getByText('Choose checks').first()).toBeVisible();

  // Map -> Choose checks
  await cta(page, 'Choose checks').click();
  // Nothing is chosen until the user picks the tables to check.
  await page.getByRole('button', { name: 'Select all', exact: true }).click();
  await expect(page.getByText('Estimated time:')).toBeVisible();

  // Choose checks -> Run
  await cta(page, 'Run').click();
  await expect(page.getByText('The run has finished.')).toBeVisible();

  // Run -> Results
  await cta(page, 'See results').click();
  await expect(page.getByText(/tables fully checked/)).toBeVisible();
}
