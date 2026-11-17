import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { completeRun } from './helpers';

test('the app has no serious or critical accessibility violations', async ({ page }) => {
  await completeRun(page, 'pass');
  // Measure the settled screen: entrance animations fade content in, and a
  // half-faded frame is not what the user reads.
  await page.waitForFunction(() =>
    document.getAnimations().every((a) => a.playState !== 'running'),
  );

  const results = await new AxeBuilder({ page }).analyze();
  const serious = results.violations.filter(
    (v) => v.impact === 'serious' || v.impact === 'critical',
  );
  expect(
    serious,
    serious
      .map((v) => `${v.id}: ${v.help} (${v.nodes.length} nodes)`)
      .join('\n'),
  ).toEqual([]);
});

/** Every step, in both appearances, after its entrance animation settles. */
for (const colorScheme of ['light', 'dark'] as const) {
  test(`every step is accessible in ${colorScheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    await page.addInitScript(() => {
      window.__dmuxForcedFixture = 'fail';
    });
    await page.goto('/');
    const settle = () =>
      page.waitForFunction(() =>
        document.getAnimations().every((a) => a.playState !== 'running'),
      );
    const check = async (where: string) => {
      await settle();
      const results = await new AxeBuilder({ page }).analyze();
      const serious = results.violations.filter(
        (v) => v.impact === 'serious' || v.impact === 'critical',
      );
      expect(
        serious,
        `${where}: ` +
          serious
            .map((v) => `${v.id}: ${v.nodes.map((n) => n.failureSummary).join(' | ')}`)
            .join('\n'),
      ).toEqual([]);
    };
    const cta = (name: string) =>
      page.getByRole('main').getByRole('button', { name, exact: true });

    await check('connect');
    for (const side of ['Old database', 'New database']) {
      await page.getByTestId(`${side}-host`).fill('127.0.0.1');
      await page.getByTestId(`${side}-database`).fill(side === 'Old database' ? 'diff' : 'demo_bank');
      await page.getByTestId(`${side}-user`).fill('dmux_reader');
    }
    const tests = page.getByRole('button', { name: 'Test connection' });
    await tests.nth(0).click();
    await tests.nth(1).click();
    await expect(page.getByText('Server version:').nth(1)).toBeVisible();
    await check('connected');
    await cta('Compare structure').click();
    await expect(cta('Map tables')).toBeVisible();
    await check('structure');
    await cta('Map tables').click();
    await expect(cta('Choose checks')).toBeVisible();
    await check('map');
    await cta('Choose checks').click();
    await expect(page.getByText('Estimated time:')).toBeVisible();
    await check('choose checks');
    await cta('Run').click();
    await expect(page.getByTestId('verdict-word')).toBeVisible();
    await check('run');
    await cta('See results').click();
    await expect(page.getByRole('heading', { name: 'Failures' })).toBeVisible();
    await check('results');
    await cta('Report').click();
    await check('report');
  });
}
