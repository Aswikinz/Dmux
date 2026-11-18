import { browser, $, $$, expect } from '@wdio/globals';

// Real-app E2E: drive the RELEASE binary (the embedded UI) through the full
// seven-step flow against the CI service databases, not the mock engine. This
// is the only test that exercises the real Tauri commands -> engine -> Postgres
// path end to end, so a false PASS can never slip through a mocked adapter.
//
// The fixture is the planted demo_bank answer key (12 missing transactions, a
// changed balance, a renamed table, etc.), so the run MUST surface FAIL, never
// PASS, and the results must show a coverage line.
//
// Two variants:
//   (a) DMUX_E2E_KEYCHAIN=1 — a working Secret Service (gnome-keyring under
//       dbus-run-session). The password typed on Connect is stored in the
//       keychain and read back by Compare structure / Run, so the full flow
//       completes. Run from the build-linux job's "Real-app E2E" step.
//   (b) no keychain — no Secret Service. A password typed on Connect and
//       proven by Test connection is kept in memory for the session (T055), so
//       Compare structure works without a Save. Run from the "Real-app E2E (no
//       keychain)" step.

const E2E_USER = process.env.DMUX_E2E_USER;
const E2E_PASSWORD = process.env.DMUX_E2E_PASSWORD;
const HAS_KEYCHAIN = process.env.DMUX_E2E_KEYCHAIN === '1';

// The planted answer-key database (testdata/demo_bank/build.sh default).
const DATABASE = 'demo_bank';

/** A Connect-screen field, keyed by the data-testid the UI already emits. */
const field = (testid: string) => $(`[data-testid="${testid}"]`);

/** The primary call-to-action button on a screen (unique within <main>). */
const cta = (name: string) => $(`//main//button[normalize-space(.)="${name}"]`);

/** Click one of the per-profile buttons (Save / Test connection, old then new). */
async function clickProfileButton(text: string, index: number): Promise<void> {
  const buttons = await $$(`//main//button[normalize-space(.)="${text}"]`);
  await buttons[index].click();
}

/** One diagnostic line so a hung run shows which step it reached. */
function step(n: number, name: string): void {
  console.log(`STEP ${n}: ${name}`);
}

async function waitForBodyText(
  predicate: (text: string) => boolean,
  timeout: number,
  what: string,
): Promise<void> {
  await browser.waitUntil(async () => predicate(await $('body').getText()), {
    timeout,
    timeoutMsg: `timed out waiting for: ${what}`,
  });
}

/** Fill one profile editor and switch it to a plaintext connection. */
async function fillProfile(side: 'Old database' | 'New database', port: string): Promise<void> {
  await field(`${side}-host`).setValue('127.0.0.1');
  await field(`${side}-port`).setValue(port);
  await field(`${side}-database`).setValue(DATABASE);
  await field(`${side}-user`).setValue(E2E_USER!);
  await field(`${side}-password`).setValue(E2E_PASSWORD!);
  // The mock servers have no TLS, so an unencrypted connection is required and
  // its reason is recorded (PLAN 6.7). Selecting it reveals the reason field.
  await field(`${side}-encryption`).selectByVisibleText('Allow unencrypted connection');
  const reason = field(`${side}-plaintext-reason`);
  await reason.waitForDisplayed({ timeout: 10_000 });
  await reason.setValue('mock database has no TLS');
}

/** Save a profile (stores the password in the keychain) and test it. */
async function saveAndTest(side: 'Old database' | 'New database', index: number): Promise<void> {
  await clickProfileButton('Save', index);
  // The keychain variant requires Save to actually store the password, so the
  // later steps can read it back. A missing "Password is in your system keychain" here
  // means the keychain did not start/unlock — the timeoutMsg says so.
  await waitForBodyText(
    (t) => (t.match(/Password is in your system keychain/g) ?? []).length >= index + 1,
    20_000,
    `${side} "Password is in your system keychain" (is gnome-keyring unlocked?)`,
  );

  await clickProfileButton('Test connection', index);
  // A ServerInfoCard appears once per tested profile.
  await waitForBodyText(
    (t) => (t.match(/Server version/g) ?? []).length >= index + 1,
    30_000,
    `${side} Test connection to report server info`,
  );
}

const runKeychainFlow = HAS_KEYCHAIN && E2E_USER && E2E_PASSWORD ? it : it.skip;

describe('Dmux real-app flow against the CI service databases', function (this: {
  timeout(ms: number): void;
}) {
  // The full-values fingerprint over demo_bank takes minutes, so raise the
  // timeout for this spec to 6 minutes. It must be set on the suite (not
  // inside the test body): WebdriverIO reads the mocha timeout when it starts
  // the test, before the body runs, so an in-body this.timeout() is too late.
  this.timeout(600_000);

  runKeychainFlow(
    'drives the release app through Connect -> Report against planted demo_bank and shows FAIL',
    async () => {
      // Wait for the app shell to mount before touching the Connect form.
      await $('main').waitForDisplayed({ timeout: 30_000 });

      // 1. Connect: fill, save (keychain) and test both profiles.
      step(1, 'Connect — fill + Save + Test connection (old)');
      await fillProfile('Old database', '5433');
      await saveAndTest('Old database', 0);

      step(2, 'Connect — fill + Save + Test connection (new)');
      await fillProfile('New database', '5434');
      await saveAndTest('New database', 1);

      step(3, 'Compare structure');
      await cta('Compare structure').click();
      await waitForBodyText(
        (t) => t.includes('Old encoding') && t.includes(DATABASE),
        60_000,
        'the structure diff (Old encoding + demo_bank)',
      );

      step(4, 'Map (needs-attention queue)');
      await cta('Map tables').click();
      await waitForBodyText(
        (t) => /matched|need attention|ignored/i.test(t),
        60_000,
        'the mapping summary (matched / needs attention)',
      );

      step(5, 'Choose checks');
      await cta('Choose checks').click();
      await waitForBodyText(
        (t) => t.includes('No tables chosen yet'),
        60_000,
        'the table chooser (nothing chosen yet)',
      );
      // Nothing is chosen until the user picks: choose every table.
      await cta('Select all').click();
      await waitForBodyText((t) => t.includes('Estimate'), 60_000, 'the check plan (Estimate)');

      step(6, 'Run');
      await cta('Run').click();
      await waitForBodyText((t) => t.includes('The run has finished'), 300_000, 'the run to finish');

      step(7, 'Results');
      await cta('See results').click();
      await waitForBodyText((t) => t.includes('tables fully checked'), 60_000, 'the results');
      const resultsText = await $('body').getText();
      // demo_bank is planted: the run must surface failures, never a clean PASS.
      expect(resultsText).toContain('Failures');
      expect(resultsText).toContain('tables fully checked');

      step(8, 'Report');
      await cta('Report').click();
      await waitForBodyText((t) => t.includes('Export PDF'), 30_000, 'the report screen');
      await expect(cta('Export PDF')).toBeDisplayed();
      await expect(cta('Export Excel')).toBeDisplayed();
    },
  );

  // Without a keychain (PLAN 4.1, T055): a password typed on Connect and
  // proven by Test connection is kept in memory for the session, so Compare
  // structure and the later steps can use it.
  const runNoKeychainFlow = !HAS_KEYCHAIN && E2E_USER && E2E_PASSWORD ? it : it.skip;
  runNoKeychainFlow(
    'completes the flow without a keychain (ask-per-run password)',
    async () => {
      await $('main').waitForDisplayed({ timeout: 30_000 });

      step(1, 'Connect — fill + Test connection (old, typed password)');
      await fillProfile('Old database', '5433');
      await clickProfileButton('Test connection', 0);
      await waitForBodyText(
        (t) => (t.match(/Server version/g) ?? []).length >= 1,
        30_000,
        'old Test connection to report server info',
      );
      // The user is told where the password lives this session.
      await waitForBodyText(
        (t) => t.includes('No system keychain'),
        10_000,
        'the in-memory password notice',
      );

      step(2, 'Connect — fill + Test connection (new, typed password)');
      await fillProfile('New database', '5434');
      await clickProfileButton('Test connection', 1);
      await waitForBodyText(
        (t) => (t.match(/Server version/g) ?? []).length >= 2,
        30_000,
        'new Test connection to report server info',
      );

      step(3, 'Compare structure (should use the ask-per-run password)');
      await cta('Compare structure').click();
      await waitForBodyText((t) => t.includes('Old encoding'), 60_000, 'the structure diff');
    },
  );
});
