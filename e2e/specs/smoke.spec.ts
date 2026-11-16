import { browser, $, expect } from '@wdio/globals';

// The one Phase 0 smoke test: the real built app opens and shows the
// placeholder screen. Linux drives WebKitGTK through WebKitWebDriver under
// Xvfb; Windows drives WebView2 through an auto-managed msedgedriver.
describe('Dmux smoke test', () => {
  it('shows the placeholder screen in the Dmux window', async () => {
    await expect(browser).toHaveTitle('Dmux');

    const heading = await $('h1');
    await expect(heading).toBeDisplayed();
    await expect(heading).toHaveText('Dmux — migration verification');

    const body = await $('body');
    await expect(body).toHaveText(expect.stringContaining('Phase 0 skeleton'));
  });
});
