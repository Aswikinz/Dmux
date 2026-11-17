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
//   (b) no keychain — expected to fail at Compare structure today, because a
//       password typed on Connect is not threaded through later steps (known
//       gap T055). Skipped with a TODO so it turns on once T055 lands.

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

async function waitForBodyText(
  predicate: (text: string) => boolean,
  timeout: number,
  what: string,
): Promise<void> {
  await browser.waitUntil(async () => predicate(await $('body').getText()), {
    timeout,
    timeoutMsg: `timed out waiting for ${what}`,
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
  // "Password saved in keychain" appears once per saved profile.
  await waitForBodyText(
    (t) => (t.match(/Password saved in keychain/g) ?? []).length >= index + 1,
    15_000,
    `${side} password to be saved`,
  );

  await clickProfileButton('Test connection', index);
  // A ServerInfoCard appears once per tested profile.
  await waitForBodyText(
    (t) => (t.match(/Server version/g) ?? []).length >= index + 1,
    30_000,
    `${side} connection to succeed`,
  );
}

const runKeychainFlow = HAS_KEYCHAIN && E2E_USER && E2E_PASSWORD ? it : it.skip;

runKeychainFlow(
  'drives the release app through Connect -> Report against planted demo_bank and shows FAIL',
  async function (this: { timeout(ms: number): void }) {
    this.timeout(600_000);

    // Wait for the app shell to mount before touching the Connect form.
    await $('main').waitForDisplayed({ timeout: 30_000 });

    // 1. Connect: fill, save (keychain) and test both profiles.
    await fillProfile('Old database', '5433');
    await saveAndTest('Old database', 0);
    await fillProfile('New database', '5434');
    await saveAndTest('New database', 1);

    await cta('Compare structure').click();
    // 2. Compare structure: the side-by-side inventory shows both encodings
    // (old LATIN1/C, new UTF-8) and the planted schema differences.
    await waitForBodyText(
      (t) => t.includes('Old encoding') && t.includes(DATABASE),
      60_000,
      'the structure diff',
    );

    // 3. Map: the planted rename/column changes surface as "needs attention";
    // exact matches are accepted by default, so move straight on.
    await cta('Map tables').click();
    await waitForBodyText(
      (t) => /matched|needs attention|ignored/i.test(t),
      60_000,
      'the mapping',
    );

    // 4. Choose checks: the default plan (full values everywhere) is fine.
    await cta('Choose checks').click();
    await waitForBodyText((t) => t.includes('Estimate'), 60_000, 'the check plan');

    // 5. Run: the engine fingerprints every table and must finish (never hang).
    await cta('Run').click();
    await waitForBodyText((t) => t.includes('Finished'), 300_000, 'the run to finish');

    // 6. Results: planted differences mean FAIL, with a coverage line — the
    // worst bug this project can have is a false PASS, so assert the run
    // surfaced failures, never a clean PASS. (The overall verdict badge reads
    // "Fail"; the Results screen only renders a "Failures" section when some
    // table did not pass, which for the planted demo_bank it always must.)
    await cta('See results').click();
    await waitForBodyText((t) => t.includes('tables fully checked'), 60_000, 'the results');
    const resultsText = await $('body').getText();
    expect(resultsText).toContain('Failures');
    expect(resultsText).toContain('tables fully checked');

    // 7. Report: the export controls exist. Export itself is NOT_AVAILABLE
    // until T024, and a native save-location dialog cannot be driven by
    // WebDriver, so assert the control rather than clicking it.
    await cta('Report').click();
    await waitForBodyText((t) => t.includes('Export PDF'), 30_000, 'the report screen');
    await expect(cta('Export PDF')).toBeDisplayed();
    await expect(cta('Export Excel')).toBeDisplayed();
  },
);

// TODO(T055): once a typed password is threaded through later steps without a
// keychain, flip `it.skip` to `it`. Until then this would fail at Compare
// structure ("no password is stored for this profile"), because the password
// typed on Connect is dropped after Test connection and only a keychain save
// makes it reach run_inventory / build_mapping / start_run.
it.skip(
  'completes the flow without a keychain (ask-per-run password)',
  async function (this: { timeout(ms: number): void }) {
    this.timeout(600_000);

    await $('main').waitForDisplayed({ timeout: 30_000 });

    await fillProfile('Old database', '5433');
    await clickProfileButton('Test connection', 0);
    await waitForBodyText(
      (t) => (t.match(/Server version/g) ?? []).length >= 1,
      30_000,
      'old connection to succeed',
    );
    await fillProfile('New database', '5434');
    await clickProfileButton('Test connection', 1);
    await waitForBodyText(
      (t) => (t.match(/Server version/g) ?? []).length >= 2,
      30_000,
      'new connection to succeed',
    );

    // T055: with no keychain the typed password is not remembered, so the next
    // step must still work from the ask-per-run password, not fail here.
    await cta('Compare structure').click();
    await waitForBodyText((t) => t.includes('Old encoding'), 60_000, 'the structure diff');
  },
);
