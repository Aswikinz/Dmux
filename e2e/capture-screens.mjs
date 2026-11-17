/* eslint-disable no-undef -- standalone Node CLI (not the browser UI), run via `node` */
// Capture the Dmux screens for the non-SQL-user walkthrough (docs/qa/).
//
// Drives the SAME React screens a real user sees, in a headless Chromium via
// Playwright, with the deterministic mock engine (the browser runtime uses the
// mock, never Tauri — see src/api/engineSelection.ts). The real-app screenshots
// (WebKitGTK, against live demo_bank) are captured by the CI smoke job's
// real-flow spec; WebKitWebDriver is not installed on this machine.
//
// Usage: node e2e/capture-screens.mjs   (starts its own Vite dev server)
import { chromium } from '@playwright/test';
import { spawn, execSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';

const PORT = 5174;
// Screenshots land next to the walkthrough report, one directory up from e2e/.
const OUT = new URL('../docs/qa/screenshots/', import.meta.url);
mkdirSync(OUT, { recursive: true });

// Clear any stale dev server from an earlier run so the new one can bind.
try {
  execSync(`fuser -k ${PORT}/tcp`, { stdio: 'ignore' });
} catch {
  /* nothing was listening */
}

// Spawn Vite directly (not through pnpm) so the process we kill is the server.
const server = spawn('./node_modules/.bin/vite', ['--port', String(PORT), '--strictPort'], {
  stdio: 'ignore',
  detached: true,
});
const cleanup = () => {
  try {
    process.kill(-server.pid, 'SIGTERM');
  } catch {
    /* already gone */
  }
};
process.on('exit', cleanup);

async function waitForServer(url) {
  for (let i = 0; i < 60; i++) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error('vite did not start');
}

await waitForServer(`http://localhost:${PORT}`);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const base = `http://localhost:${PORT}`;
const shot = (name) => page.screenshot({ path: new URL(`./${name}.png`, OUT).pathname });

const cta = (name) => page.getByRole('main').getByRole('button', { name, exact: true });

async function fillProfiles(database) {
  for (const [side, port] of [
    ['Old database', '5433'],
    ['New database', '5434'],
  ]) {
    await page.getByTestId(`${side}-host`).fill('127.0.0.1');
    await page.getByTestId(`${side}-port`).fill(port);
    await page.getByTestId(`${side}-database`).fill(database);
    await page.getByTestId(`${side}-user`).fill('dmux_dev');
    await page.getByTestId(`${side}-password`).fill('dmux_dev_only');
    await page.getByTestId(`${side}-encryption`).selectOption('allow-plaintext');
    await page.getByTestId(`${side}-plaintext-reason`).fill('mock database has no TLS');
  }
}

async function testBoth() {
  const buttons = page.getByRole('button', { name: 'Test connection' });
  await buttons.nth(0).click();
  await page.getByText('Server version:').first().waitFor();
  await buttons.nth(1).click();
  await page.getByText('Server version:').nth(1).waitFor();
}

// 1. Connect, empty.
await page.goto(base);
await page.getByRole('heading', { name: 'Connect' }).waitFor();
await shot('01-connect-empty');

// 2. Connect, tested (mock server info shown).
await fillProfiles('demo_bank');
await testBoth();
await shot('02-connect-tested');

// 3. Compare structure (identical demo_bank shape).
await cta('Compare structure').click();
await page.getByText('Old encoding:').waitFor();
await shot('03-structure');

// 4. Compare structure, differences (red/green/amber). Fresh page, 'diff' db.
await page.goto(base);
await fillProfiles('diff');
await testBoth();
await cta('Compare structure').click();
await page.getByText('Old encoding:').waitFor();
await shot('04-structure-diff');

// 5. Map (needs-attention queue from the diff).
await cta('Map tables').click();
await page.getByText(/need attention/i).waitFor();
await shot('05-map');

// 6. Choose checks.
await cta('Choose checks').click();
await page.getByText('Estimate:').waitFor();
await shot('06-choose-checks');

// 7. Run (finished).
await cta('Run').click();
await page.getByText('Finished').waitFor();
await shot('07-run');

// 8. Results, FAIL (forced fail fixture with planted differences).
await page.addInitScript(() => {
  window.__dmuxForcedFixture = 'fail';
});
await page.goto(base);
await fillProfiles('demo_bank');
await testBoth();
await cta('Compare structure').click();
await page.getByText('Old encoding:').waitFor();
await cta('Map tables').click();
await page.getByText(/need attention/i).waitFor();
await cta('Choose checks').click();
await page.getByText('Estimate:').waitFor();
await cta('Run').click();
await page.getByText('Finished').waitFor();
await cta('See results').click();
await page.getByText(/tables fully checked/).waitFor();
await shot('08-results-fail');

// 9. Report.
await cta('Report').click();
await page.getByRole('button', { name: /Export PDF/ }).waitFor();
await shot('09-report');

await browser.close();
cleanup();
console.log('screenshots written to', OUT.pathname);
