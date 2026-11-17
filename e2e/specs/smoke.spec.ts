import { browser, $, expect } from '@wdio/globals';

// The one smoke test: the real built app opens and shows the guided flow.
// Linux drives WebKitGTK through WebKitWebDriver under Xvfb; Windows drives
// WebView2 through an auto-managed msedgedriver.
describe('Dmux smoke test', () => {
  it('opens the window and shows the Connect screen', async () => {
    await expect(browser).toHaveTitle('Dmux');

    // Stable shell strings (post-T020): the sidebar brand and the first
    // screen's heading, both present on launch before any connection.
    const brand = await $('aside .text-xl');
    await expect(brand).toBeDisplayed();
    await expect(brand).toHaveText('Dmux');

    const heading = await $('h2');
    await expect(heading).toBeDisplayed();
    await expect(heading).toHaveText('Connect');
  });
});