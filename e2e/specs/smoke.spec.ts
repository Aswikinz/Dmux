import { browser, $, expect } from '@wdio/globals';

// The one smoke test: the real built app opens and shows the shipped shell.
// It checks rendered content, not just that a window appeared, so a broken
// bundle or a missing embedded asset fails CI. Linux drives WebKitGTK through
// WebKitWebDriver under Xvfb; Windows drives WebView2 through an auto-managed
// msedgedriver.
describe('Dmux smoke test', () => {
  it('shows the app shell and the Connect screen in the Dmux window', async () => {
    await expect(browser).toHaveTitle('Dmux');

    const main = await $('main');
    await expect(main).toBeDisplayed();

    const heading = await $('//main//h2[normalize-space(.)="Connect"]');
    await expect(heading).toBeDisplayed();

    const body = await $('body');
    await expect(body).toHaveText(expect.stringContaining('Old database'));
  });
});
