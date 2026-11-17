/**
 * Pick the right engine for the current runtime.
 *
 * - Inside the Tauri desktop app the real engine (`createTauriEngine`) talks to
 *   the Rust backend for the structure half; the data half is stubbed (throws).
 * - In a browser / Vitest / Playwright the mock is used, so UI tests stay
 *   deterministic and never touch a database.
 * - A test may force a mock fixture through `window.__dmuxForcedFixture`, but
 *   only outside Tauri and never in a production build.
 */

import { createMockEngine, type MockEngineOptions } from './mockEngine';
import { createTauriEngine } from './tauriEngine';
import type { Engine } from './engine';

/**
 * Is the current runtime a Tauri desktop window?
 *
 * `@tauri-apps/api` injects `window.__TAURI_INTERNALS__` in the built app; it is
 * absent in the browser (Vitest / Playwright).
 */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/**
 * Whether a test may force a mock fixture.  True only in a non-production build
 * (`import.meta.env.MODE === 'test'` or a dev server) AND outside Tauri.
 */
export function mayForceFixture(): boolean {
  if (isTauri()) {
    return false;
  }
  if (import.meta.env.PROD) {
    return false;
  }
  const mode = import.meta.env.MODE;
  return mode === 'test' || mode === 'development';
}

/** The mock fixture forced by a test harness (only honoured when allowed). */
export function forcedFixture(): string | undefined {
  if (typeof window === 'undefined') {
    return undefined;
  }
  if (!mayForceFixture()) {
    return undefined;
  }
  return (window as unknown as { __dmuxForcedFixture?: string }).__dmuxForcedFixture;
}

/**
 * Resolve the engine for the current runtime.
 *
 * Order of precedence:
 *   1. a forced fixture (test/dev only, never inside Tauri) → deterministic mock;
 *   2. inside Tauri → the real engine (data half stubbed until T022);
 *   3. otherwise (browser / Vitest / Playwright) → the deterministic mock.
 */
export function resolveEngine(): Engine {
  const fixture = forcedFixture();
  if (fixture !== undefined) {
    return createMockEngine({ forcedFixture: fixture as MockEngineOptions['forcedFixture'] });
  }
  if (isTauri()) {
    return createTauriEngine();
  }
  return createMockEngine();
}
