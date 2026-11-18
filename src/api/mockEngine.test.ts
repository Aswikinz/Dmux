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
  it('a run reads only the chosen tables and records the rest as not selected', async () => {
    const engine = createMockEngine();
    const plan = await engine.buildCheckPlan(mapping());
    const tables = plan.tables.map((t, i) => ({ ...t, included: i < 3 }));
    const started = await engine.startRun({ ...definition(), plan: { ...plan, tables } });
    const progress = await engine.pollRun(started.runId);
    expect(progress.tables.map((t) => t.tableId)).toEqual(plan.tables.slice(0, 3).map((t) => t.tableId));
    const results = await engine.getResults(started.runId);
    const left = results.ignored.filter((i) => i.reason === 'not selected for this run');
    expect(left).toHaveLength(37);
    expect(left[0].name).toBe(`${plan.tables[3].schemaName}.${plan.tables[3].tableName}`);
  });

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

  it('says plainly that the browser demo saves no mapping file', async () => {
    const saved = await mockEngine.saveMapping(mapping());
    expect(saved.ok).toBe(false);
    expect(saved.demoOnly).toBe(true);
    expect(saved.hash).toMatch(/^mock-/);
    expect(await mockEngine.saveMapping(mapping())).toEqual(saved);
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
      mockEngine.exportReport({ runId: 'run-1', format: 'pdf', includeKeys: false }),
    ).resolves.toEqual({ ok: false, demoOnly: true });
    await expect(
      mockEngine.exportReport({ runId: 'run-1', format: 'excel', includeKeys: true }),
    ).resolves.toEqual({ ok: false, demoOnly: true });
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

describe('mock profiles and history', () => {
  it('lists saved profiles and forgets a deleted one with its password', async () => {
    const engine = createMockEngine();
    const profile = {
      id: 'p1',
      label: 'B',
      host: 'h',
      port: 5432,
      database: 'd',
      user: 'u',
      encryption: 'verify' as const,
    };
    await engine.saveProfile(profile, 'x');
    await engine.saveProfile({ ...profile, id: 'p0', label: 'A' });
    expect((await engine.listProfiles()).map((p) => p.id)).toEqual(['p0', 'p1']);
    expect(await engine.deleteProfile('p1')).toBe(true);
    expect(await engine.deleteProfile('p1')).toBe(false);
    expect(await engine.profileHasPassword('p1')).toBe(false);
  });

  it('keeps a history that can be opened, logged and purged', async () => {
    const engine = createMockEngine();
    const rows = await engine.listHistory();
    expect(rows.length).toBe(2);
    const opened = await engine.openHistoryEntry(rows[0].id);
    expect(opened.verdict).toBe(rows[0].verdict);
    await expect(engine.openHistoryEntry('nope')).rejects.toThrow();
    expect(await engine.getHistoryLog(rows[0].id)).toContain('Dmux run log');
    expect(await engine.saveHistoryLog(rows[0].id)).toEqual({ ok: false, demoOnly: true });
    expect(await engine.purgeHistory()).toBe(2);
    expect(await engine.listHistory()).toEqual([]);
  });

  it('opens a results file, or reports a dismissed or refused one', async () => {
    expect((await createMockEngine().openResultsFile()).results?.verdict).toBe('FAIL');
    expect(await createMockEngine({ openFile: 'cancelled' }).openResultsFile()).toEqual({
      ok: false,
      cancelled: true,
    });
    const refused = await createMockEngine({ openFile: 'tampered' }).openResultsFile();
    expect(refused.ok).toBe(false);
    expect(refused.results).toBeUndefined();
  });
});
