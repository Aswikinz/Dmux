/**
 * Pick the right engine for the current runtime.
 *
 * - Inside the Tauri desktop app the real engine (`createTauriEngine`) talks to
 *   the Rust backend.  The data half still delegates to the mock until T022.
 * - In a browser / Vitest / Playwright the mock is used, so UI tests stay
 *   deterministic and never touch a database.
 * - A test may force a mock fixture through `window.__dmuxForcedFixture`.
 */

import { createMockEngine, type MockEngineOptions } from './mockEngine';
import { createTauriEngine } from './tauriEngine';
import type { Engine } from './engine';

/**
 * Is the current runtime a Tauri desktop window?
 *
 * `@tauri-apps/api` injects `window.__TAURI_INTERNALS__` in the built app; it is
 * absent in the browser (Vitest / Playwright) and in `tauri dev`'s served page
 * is guarded separately by `isTauri`.
 */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The mock fixture forced by a test harness, if any. */
export function forcedFixture(): string | undefined {
  if (typeof window === 'undefined') {
    return undefined;
  }
  return (window as unknown as { __dmuxForcedFixture?: string }).__dmuxForcedFixture;
}

/**
 * Resolve the engine for the current runtime.
 *
 * Order of precedence:
 *   1. a forced fixture (Playwright test) → deterministic mock;
 *   2. inside Tauri → the real engine (data half still on the mock);
 *   3. otherwise (browser / Vitest) → the deterministic mock.
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