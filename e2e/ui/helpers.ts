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

/** Connect both databases on the Connect screen and move to the next step. */
export async function connect(page: Page): Promise<void> {
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
  await expect(page.getByText('Estimate:')).toBeVisible();

  // Choose checks -> Run
  await cta(page, 'Run').click();
  await expect(page.getByText('Finished')).toBeVisible();

  // Run -> Results
  await cta(page, 'See results').click();
  await expect(page.getByText(/tables fully checked/)).toBeVisible();
}
