import { afterEach, describe, expect, it, vi } from 'vitest';
import { isTauri, forcedFixture, resolveEngine } from './engineSelection';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

afterEach(() => {
  // Restore a clean window between tests.
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  delete (window as unknown as Record<string, unknown>).__dmuxForcedFixture;
});

describe('isTauri', () => {
  it('is false without the Tauri internals', () => {
    expect(isTauri()).toBe(false);
  });

  it('is true when the Tauri internals are present', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(isTauri()).toBe(true);
  });
});

describe('resolveEngine', () => {
  it('returns the mock in the browser (no Tauri internals)', () => {
    const engine = resolveEngine();
    expect(engine).toBeDefined();
    // The mock returns a deterministic mock version, not the backend version.
    return engine.getVersion().then((v) => {
      expect(v).toContain('mock');
    });
  });

  it('returns the real engine inside Tauri', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    const engine = resolveEngine();
    // The real engine's getVersion reads from the backend via invoke.
    invoke.mockResolvedValueOnce('0.1.0');
    return engine.getVersion().then((v) => {
      expect(v).toBe('0.1.0');
    });
  });

  it('honours a forced fixture even inside Tauri', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    (window as unknown as Record<string, unknown>).__dmuxForcedFixture = 'fail';
    const engine = resolveEngine();
    // A forced fixture routes to the mock regardless of the Tauri runtime.
    return engine.getVersion().then((v) => {
      expect(v).toContain('mock');
    });
  });
});

describe('forcedFixture', () => {
  it('reads the window override', () => {
    (window as unknown as Record<string, unknown>).__dmuxForcedFixture = 'pass';
    expect(forcedFixture()).toBe('pass');
  });

  it('is undefined when not set', () => {
    expect(forcedFixture()).toBeUndefined();
  });
});