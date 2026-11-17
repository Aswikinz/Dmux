import path from 'node:path';
import { fileURLToPath } from 'node:url';
import type { TauriCapabilities } from '@wdio/tauri-service';

// Paths are resolved from this config file, so the run works from any cwd.
// The cargo workspace shares one target/ directory at the repo root.
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
// Drive the RELEASE binary: only a release build (`pnpm tauri build`)
// embeds the UI. A plain `cargo build` produces a dev-mode app that loads
// http://localhost:5173 at runtime — in CI nothing serves it, and locally
// it would silently test the dev server instead of the shipped app.
// DMUX_APP_BINARY overrides the path (relative paths resolve from the root).
const appBinary = process.env.DMUX_APP_BINARY
  ? path.resolve(root, process.env.DMUX_APP_BINARY)
  : path.join(
      root,
      'target',
      'release',
      process.platform === 'win32' ? 'dmux-app.exe' : 'dmux-app',
    );

const capabilities: TauriCapabilities[] = [
  {
    browserName: 'tauri',
    'tauri:options': {
      application: appBinary,
    },
  },
];

export const config: WebdriverIO.Config = {
  runner: 'local',
  specs: ['./specs/**/*.spec.ts'],
  maxInstances: 1,
  capabilities,
  services: [
    [
      '@wdio/tauri-service',
      {
        // Decision 7: the upstream tauri-driver, never a fork. 'external'
        // is the provider name for it in @wdio/tauri-service 1.5.1;
        // 'official' is a deprecated alias, removed in v2.
        driverProvider: 'external',
        appBinaryPath: appBinary,
        // CI installs tauri-driver pinned via cargo; locally install it yourself.
        autoInstallTauriDriver: false,
        // Windows only: match msedgedriver to the app's WebView2 version.
        autoDownloadEdgeDriver: true,
        tauriDriverPort: 4444,
        startTimeout: 60000,
      },
    ],
  ],
  logLevel: 'warn',
  bail: 1,
  waitforTimeout: 10000,
  connectionRetryTimeout: 120000,
  connectionRetryCount: 3,
  framework: 'mocha',
  mochaOpts: {
    ui: 'bdd',
    timeout: 120000,
  },
  reporters: ['spec'],
  // On a failure, capture a screenshot and the rendered page text so a hung or
  // failed real-app run is debuggable from CI (uploaded as an artifact by
  // ci.yml). demo_bank only — the page text is truncated and never contains the
  // typed password (the UI clears it immediately and never renders it).
  // @wdio/tauri-service 1.5.1 runs a window-focus check before every
  // findElement/click that calls into its optional Tauri plugin; we do not ship
  // that plugin, so each check waits ~5 s for an invoke timeout. An explicit
  // switchToWindow marks the session as "user switched" and turns the check off
  // (see afterCommand in the service). Dmux has a single window.
  before: async () => {
    await browser.switchToWindow(await browser.getWindowHandle());
  },
  afterTest: async (test, _context, result) => {
    if (!result.error) {
      return;
    }
    const fs = await import('node:fs');
    const path = await import('node:path');
    const outDir = path.resolve('e2e', 'artifacts');
    fs.mkdirSync(outDir, { recursive: true });
    const slug = test.title.replace(/[^a-zA-Z0-9]+/g, '_').slice(0, 80) || 'unknown';
    try {
      await browser.saveScreenshot(path.join(outDir, `${slug}.png`));
    } catch {
      // The WebDriver session may already be gone after a timeout; that is fine.
    }
    try {
      const text = (await browser.execute('return document.body.innerText')) as unknown as string;
      fs.writeFileSync(path.join(outDir, `${slug}.txt`), text.slice(0, 4096));
    } catch {
      // Ditto: the session may be gone, so there is nothing to dump.
    }
  },
};
