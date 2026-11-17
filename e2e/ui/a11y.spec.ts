import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { completeRun } from './helpers';

test('the app has no serious or critical accessibility violations', async ({ page }) => {
  await completeRun(page, 'pass');

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
