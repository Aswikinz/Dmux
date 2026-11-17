import { describe, expect, it } from 'vitest';
import {
  createMockEngine,
  mockEngine,
  fixtures,
  fixtureNames,
} from './mockEngine';
import type { ConnectionProfile, Mapping, RunDefinition } from './contract';
import { defaultOldProfile, defaultNewProfile } from '../screens/ConnectScreen';

function profile(overrides: Partial<ConnectionProfile> = {}): ConnectionProfile {
  return { ...defaultOldProfile(), ...overrides };
}

function mapping(): Mapping {
  return { rules: [], matches: [], needsAttention: [], ignored: [] };
}

function definition(): RunDefinition {
  return {
    oldProfile: defaultOldProfile(),
    newProfile: defaultNewProfile(),
    mapping: mapping(),
    plan: { tables: [], presets: [], estimateSecondsMin: 1, estimateSecondsMax: 2 },
  };
}

describe('mockEngine', () => {
  it('reports an error for an unreachable host', async () => {
    const res = await mockEngine.testConnection(profile({ host: 'error' }));
    expect(res.ok).toBe(false);
    expect(res.error).toBeDefined();
  });

  it('refuses superuser logins until the user allows them with a reason', async () => {
    const engine = createMockEngine();
    const refused = await engine.testConnection(profile({ user: 'superuser' }));
    expect(refused.ok).toBe(false);
    expect(refused.superuserRefused).toBe(true);
    engine.setSuperuserOverride({ reason: 'rehearsal copy' });
    const res = await engine.testConnection(profile({ user: 'superuser' }));
    expect(res.ok).toBe(true);
    expect(res.info?.isSuperuser).toBe(true);
    expect(res.info?.canWrite).toBe(true);
    expect(res.info?.warning).toContain('rehearsal copy');
  });

  it('warns on plaintext connections', async () => {
    const res = await mockEngine.testConnection(profile({ encryption: 'allow-plaintext' }));
    expect(res.ok).toBe(true);
    expect(res.info?.tls.state).toBe('plaintext');
  });

  it('reports an unverified fingerprint for trust mode', async () => {
    const res = await mockEngine.testConnection(profile({ encryption: 'trust' }));
    expect(res.info?.tls).toMatchObject({ verified: false });
    expect(res.info?.tls).toHaveProperty('fingerprint');
  });

  it('accepts a private-CA file on a verify profile', async () => {
    const res = await mockEngine.testConnection(profile({ caFile: '/tmp/bank-ca.pem' }));
    expect(res.ok).toBe(true);
    expect(res.info?.tls).toEqual({ state: 'encrypted', verified: true });
  });

  it('saveProfile stores the password in its (mock) keychain and profileHasPassword reports it', async () => {
    const engine = createMockEngine();
    await engine.saveProfile(profile(), 'hunter2');
    expect(await engine.profileHasPassword(defaultOldProfile().id)).toBe(true);
    expect(await engine.profileHasPassword(defaultNewProfile().id)).toBe(false);
  });

  it('saveProfile without a password does not store one', async () => {
    const engine = createMockEngine();
    await engine.saveProfile(profile());
    expect(await engine.profileHasPassword(defaultOldProfile().id)).toBe(false);
  });

  it('returns the base inventory by default', async () => {
    const inv = await mockEngine.compareStructure(defaultOldProfile(), defaultNewProfile());
    expect(inv.schemas[0].name).toBe('demo_bank');
  });

  it('returns the diff inventory when a database is named diff', async () => {
    const inv = await mockEngine.compareStructure(
      profile({ database: 'diff' }),
      defaultNewProfile(),
    );
    const changed = inv.schemas[0].children?.flatMap((t) => t.children ?? []).find((c) => c.status === 'changed');
    expect(changed).toBeDefined();
  });

  it('builds a deterministic mapping', async () => {
    const inv = await mockEngine.compareStructure(defaultOldProfile(), defaultNewProfile());
    const mapping = await mockEngine.buildMapping(inv);
    expect(mapping.matches.length).toBeGreaterThan(0);
  });

  it('saves a mapping to a stable hash and JSON', async () => {
    const { hash, json } = await mockEngine.saveMapping(mapping());
    expect(hash).toMatch(/^mock-/);
    expect(JSON.parse(json)).toEqual(mapping());
  });

  it('builds a check plan with 40 tables', async () => {
    const plan = await mockEngine.buildCheckPlan(mapping());
    expect(plan.tables.length).toBe(40);
    expect(plan.estimateSecondsMin).toBeLessThanOrEqual(plan.estimateSecondsMax);
  });

  it('starts, polls and cancels a run', async () => {
    const engine = createMockEngine();
    const start = await engine.startRun(definition());
    expect(start.overallState).toBe('done');
    const polled = await engine.pollRun(start.runId);
    expect(polled.tables.length).toBe(40);
    const paused = await engine.setRunPaused(start.runId, true);
    expect(paused.overallState).toBe('paused');
    const cancelled = await engine.cancelRun(start.runId);
    expect(cancelled.overallState).toBe('cancelled');
  });

  it('exports PDF and Excel reports', async () => {
    await expect(
      mockEngine.exportReport({ format: 'pdf', includeKeys: false, filePath: '/tmp/x' }),
    ).resolves.toMatchObject({ ok: true, filePath: '/tmp/x/report.pdf' });
    await expect(
      mockEngine.exportReport({ format: 'excel', includeKeys: true, filePath: '/tmp/x' }),
    ).resolves.toMatchObject({ ok: true, filePath: '/tmp/x/report.xlsx' });
  });

  it('returns its version string', async () => {
    expect(await mockEngine.getVersion()).toMatch(/mock/);
  });

  it('honours a forced fixture across calls', async () => {
    const engine = createMockEngine({ forcedFixture: 'fail' });
    const results = await engine.getResults('run-anything');
    expect(results.verdict).toBe('FAIL');
    expect(results.failures.length).toBeGreaterThan(0);
  });
});

describe('fixtures', () => {
  it.each(fixtureNames)('produces a stable result for %s', (name) => {
    const results = fixtures[name];
    expect(results.runId).toBeDefined();
    expect(results.coverage.totalTables).toBeGreaterThan(0);
    expect(results.tableResults.length).toBe(results.coverage.totalTables);
  });

  it('distinguishes every verdict', () => {
    expect(fixtures.pass.verdict).toBe('PASS');
    expect(fixtures.fail.verdict).toBe('FAIL');
    expect(fixtures.widelyDifferent.verdict).toBe('WIDELY_DIFFERENT');
    expect(fixtures.needsReview.verdict).toBe('NEEDS_REVIEW');
    expect(fixtures.changedDuringCheck.verdict).toBe('CHANGED_DURING_CHECK');
    expect(fixtures.toolFault.verdict).toBe('TOOL_FAULT');
    expect(fixtures.warning.verdict).toBe('WARNING');
  });

  it('never folds widely-different or tool-fault into FAIL', () => {
    expect(fixtures.widelyDifferent.verdict).not.toBe('FAIL');
    expect(fixtures.toolFault.verdict).not.toBe('FAIL');
  });

  it('reports failure cards with their own verdict', () => {
    expect(fixtures.widelyDifferent.failures[0].summary).toMatch(/buckets differ/);
    expect(fixtures.widelyDifferent.failures[0].verdict).toBe('WIDELY_DIFFERENT');
    expect(fixtures.toolFault.failures[0].summary).toMatch(/spot check/);
    expect(fixtures.toolFault.failures[0].verdict).toBe('TOOL_FAULT');
    expect(fixtures.needsReview.failures[0].summary).toMatch(/manual review/);
    expect(fixtures.needsReview.failures[0].verdict).toBe('NEEDS_REVIEW');
  });

  it('lists every ignored item with a reason', () => {
    expect(fixtures.pass.ignored.length).toBe(fixtures.pass.coverage.ignoredTables);
    for (const item of fixtures.pass.ignored) {
      expect(item.reason.trim().length).toBeGreaterThan(0);
    }
  });
});
