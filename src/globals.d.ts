export {};

declare global {
  interface Window {
    /** Used by Playwright tests to force a particular mock fixture. */
    __dmuxForcedFixture?: string;
  }
}
