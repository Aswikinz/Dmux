import { defineConfig, devices } from '@playwright/test';

// DMUX_UI_PORT lets several working copies run the UI tests at the same time.
const port = Number(process.env.DMUX_UI_PORT ?? 5174);

// Browser UI tests drive the React app (engine mocked via a global). They are
// separate from the Tauri smoke test under e2e/specs (WebdriverIO), so they get
// their own testDir and web server on a distinct port.
export default defineConfig({
  testDir: './e2e/ui',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: `http://localhost:${port}`,
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: `pnpm exec vite --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
