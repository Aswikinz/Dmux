import { afterEach, describe, expect, it, vi } from 'vitest';
import { isTauri, mayForceFixture, forcedFixture, resolveEngine } from './engineSelection';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

afterEach(() => {
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

describe('forcedFixture gating', () => {
  it('is honoured in a test/dev build outside Tauri', () => {
    // Vitest runs with MODE === 'test' and PROD === false.
    (window as unknown as Record<string, unknown>).__dmuxForcedFixture = 'fail';
    expect(mayForceFixture()).toBe(true);
    expect(forcedFixture()).toBe('fail');
  });

  it('is ignored inside Tauri, even in a test build', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    (window as unknown as Record<string, unknown>).__dmuxForcedFixture = 'fail';
    expect(mayForceFixture()).toBe(false);
    expect(forcedFixture()).toBeUndefined();
  });

  it('is undefined when never set', () => {
    expect(forcedFixture()).toBeUndefined();
  });
});

describe('resolveEngine', () => {
  it('returns the mock in the browser (no Tauri internals)', () => {
    const engine = resolveEngine();
    return engine.getVersion().then((v) => {
      expect(v).toContain('mock');
    });
  });

  it('returns the real engine inside Tauri', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    const engine = resolveEngine();
    invoke.mockResolvedValueOnce('0.1.0');
    return engine.getVersion().then((v) => {
      expect(v).toBe('0.1.0');
    });
  });

  it('ignores a forced fixture inside Tauri and uses the real engine', () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    (window as unknown as Record<string, unknown>).__dmuxForcedFixture = 'fail';
    const engine = resolveEngine();
    invoke.mockResolvedValueOnce('0.1.0');
    return engine.getVersion().then((v) => {
      expect(v).toBe('0.1.0'); // real engine, not the forced mock
    });
  });
});