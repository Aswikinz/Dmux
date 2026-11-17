import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createTauriEngine,
  EngineError,
  mapDiffStatus,
  mapLevel,
  mapOverallState,
  mapTableState,
  mapVerdict,
  toEngineLevel,
} from './tauriEngine';
import type { ConnectionProfile, Inventory, Mapping } from './contract';
import { defaultOldProfile } from '../screens/ConnectScreen';

// Mock the Tauri invoke bridge; the tests assert the exact command names and
// arg shapes, and the adapter's strict mappings.
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

function profile(overrides: Partial<ConnectionProfile> = {}): ConnectionProfile {
  return { ...defaultOldProfile(), ...overrides };
}

const rawServerInfo = {
  version_num: 170000,
  version_string: '17.0',
  encoding: 'UTF8',
  collation: 'en_US.utf8',
  ctype: 'en_US.utf8',
  lc_monetary: 'en_US.utf8',
  is_superuser: false,
  can_write: false,
  pooler_suspected: false,
  md5_available: true,
  sql_ascii: false,
  tls: { state: 'encrypted', verified: true },
};

const emptyInventory: Record<string, unknown> = {};

function rawDiff(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    status: 'equal',
    old_encoding: 'UTF8',
    new_encoding: 'UTF8',
    old_collation: 'C',
    new_collation: 'C',
    entries: [],
    ...overrides,
  };
}

function rawMappingBuild(): {
  mapping: Record<string, unknown>;
  needs_attention: Array<Record<string, unknown>>;
  suggestions: Array<Record<string, unknown>>;
} {
  return {
    mapping: {
      version: 1,
      rules: [{ kind: 'prefix_replace', pattern: 'old_', target: 'new_' }],
      schema_mappings: {
        public: {
          target_schema: 'public',
          table_mappings: {
            accounts: {
              target_table: 'accounts',
              status: 'exact',
              column_mappings: {
                id: { target_column: 'id', status: 'exact' },
                name: { target_column: 'full_name', status: 'case_insensitive' },
                notes: { target_column: 'memo', status: 'fuzzy' },
              },
            },
            legacy: { target_table: '', status: 'ignored', column_mappings: {} },
          },
        },
      },
      ignored: [{ path: 'public.legacy', reason: 'not migrated' }],
      mapping_hash: 'abc123',
    },
    needs_attention: [{ path: 'public.audit_log', issue: 'missing_in_new', candidates: [] }],
    suggestions: [{ path: 'public.acounts', candidate: 'accounts', confidence: 90 }],
  };
}

beforeEach(() => {
  invoke.mockReset();
});

describe('mapVerdict', () => {
  it('maps every known engine verdict 1:1', () => {
    const expected = [
      'PASS',
      'FAIL',
      'WIDELY_DIFFERENT',
      'NEEDS_REVIEW',
      'CHANGED_DURING_CHECK',
      'TOOL_FAULT',
      'WARNING',
    ];
    for (const v of expected) {
      expect(mapVerdict(v)).toBe(v);
    }
  });

  it('treats an unknown verdict as TOOL_FAULT, never PASS', () => {
    expect(mapVerdict('PASS_ALL_GOOD')).toBe('TOOL_FAULT');
    expect(mapVerdict('')).toBe('TOOL_FAULT');
    expect(mapVerdict(42)).toBe('TOOL_FAULT');
    expect(mapVerdict(null)).toBe('TOOL_FAULT');
    expect(mapVerdict(undefined)).toBe('TOOL_FAULT');
  });
});

describe('mapDiffStatus', () => {
  it('maps equal to same', () => {
    expect(mapDiffStatus('equal', '')).toBe('same');
  });

  it('maps added / missing from the message', () => {
    expect(mapDiffStatus('warning', 'column added in new: memo')).toBe('added');
    expect(mapDiffStatus('different', 'column missing in new: id')).toBe('missing');
  });

  it('maps different and warning to changed (never same)', () => {
    expect(mapDiffStatus('different', 'type: json -> jsonb')).toBe('changed');
    expect(mapDiffStatus('warning', 'type: varchar(50) -> varchar(100)')).toBe('changed');
  });
});

describe('createTauriEngine', () => {
  it('testConnection surfaces server info and the real TLS state on success', async () => {
    invoke.mockResolvedValueOnce(rawServerInfo);
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());

    expect(invoke).toHaveBeenCalledWith('probe_connection', {
      input: expect.objectContaining({ password: null }),
    });
    expect(result.ok).toBe(true);
    expect(result.info?.version).toBe('17.0');
    expect(result.info?.tls).toEqual({ state: 'encrypted', verified: true });
  });

  it('testConnection reflects a pinned certificate fingerprint', async () => {
    invoke.mockResolvedValueOnce({
      ...rawServerInfo,
      tls: { state: 'encrypted', verified: false, fingerprint: 'aabbcc' },
    });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.info?.tls).toEqual({
      state: 'encrypted',
      verified: false,
      fingerprint: 'aabbcc',
    });
  });

  it('testConnection reflects a plaintext connection with its reason', async () => {
    invoke.mockResolvedValueOnce({
      ...rawServerInfo,
      tls: { state: 'plaintext', reason: 'test network' },
    });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.info?.tls).toEqual({ state: 'plaintext', reason: 'test network' });
  });

  it('testConnection flags a superuser with a warning', async () => {
    invoke.mockResolvedValueOnce({ ...rawServerInfo, is_superuser: true });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.info?.isSuperuser).toBe(true);
    expect(result.info?.warning).toContain('superuser');
  });

  it('testConnection maps trust and plaintext profiles to the backend', async () => {
    invoke.mockResolvedValue(rawServerInfo);
    const engine = createTauriEngine();
    await engine.testConnection(
      profile({ encryption: 'allow-plaintext', plaintextReason: 'test net' }),
    );
    expect(invoke).toHaveBeenCalledWith(
      'probe_connection',
      expect.objectContaining({
        input: expect.objectContaining({
          profile: expect.objectContaining({
            encryption: 'allow-plaintext',
            plaintext_reason: 'test net',
          }),
        }),
      }),
    );
  });

  it('testConnection surfaces the untrusted-cert fingerprint', async () => {
    invoke.mockRejectedValueOnce({ kind: 'untrusted_cert', fingerprint: 'aabbcc' });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.ok).toBe(false);
    expect(result.error).toContain('fingerprint=aabbcc');
  });

  it('testConnection reports the superuser refusal distinctly', async () => {
    invoke.mockRejectedValueOnce({ kind: 'superuser_refused' });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.ok).toBe(false);
    expect(result.error).toContain('superuser');
  });

  it('testConnection reports a plaintext-reason refusal', async () => {
    invoke.mockRejectedValueOnce({ kind: 'plaintext_reason_required' });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.ok).toBe(false);
    expect(result.error).toContain('reason');
  });

  it('testConnection surfaces a generic tagged error message', async () => {
    invoke.mockRejectedValueOnce({ kind: 'database', message: 'connection failed' });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.ok).toBe(false);
    expect(result.error).toBe('connection failed');
  });

  it('testConnection turns non-object throws into a readable string', async () => {
    invoke.mockRejectedValueOnce(new Error('boom'));
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.error).toContain('boom');

    invoke.mockRejectedValueOnce('plain string');
    const result2 = await engine.testConnection(profile());
    expect(result2.error).toBe('plain string');
  });

  it('compareStructure collects both inventories and translates the diff', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'run_inventory') {
        return Promise.resolve(emptyInventory);
      }
      if (cmd === 'schema_diff') {
        return Promise.resolve(
          rawDiff({
            entries: [
              {
                path: ['schema', 'public', 'table', 'accounts'],
                status: 'warning',
                old_value: null,
                new_value: null,
                message: 'table accounts changed',
                children: [],
              },
              {
                path: ['schema', 'public', 'table', 'archived'],
                status: 'different',
                old_value: null,
                new_value: null,
                message: 'table missing in new: archived',
                children: [],
              },
            ],
            old_encoding: 'LATIN1',
            new_encoding: 'UTF8',
          }),
        );
      }
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const engine = createTauriEngine();
    const inventory = await engine.compareStructure(profile(), profile());

    expect(inventory.oldEncoding).toBe('LATIN1');
    expect(inventory.newEncoding).toBe('UTF8');
    expect(inventory.schemas).toHaveLength(2);
    const missing = inventory.schemas.find((n) => n.name === 'archived');
    expect(missing?.status).toBe('missing');
  });

  it('buildMapping requires compareStructure first', async () => {
    const engine = createTauriEngine();
    await expect(engine.buildMapping(inventory())).rejects.toThrow();
  });

  it('buildMapping only auto-approves exact and case_insensitive matches', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'run_inventory') {
        return Promise.resolve(emptyInventory);
      }
      if (cmd === 'schema_diff') {
        return Promise.resolve(rawDiff());
      }
      if (cmd === 'build_mapping') {
        return Promise.resolve(rawMappingBuild());
      }
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const engine = createTauriEngine();
    const structure = await engine.compareStructure(profile(), profile());
    const mapping = await engine.buildMapping(structure);

    // matches: accounts table (exact) + id (exact) + name (case) + notes (fuzzy).
    expect(mapping.matches).toHaveLength(4);
    const byId = (id: string) => mapping.matches.find((m) => m.id === id);

    // Exact and case-insensitive are approved, fuzzy is not and is in the queue.
    expect(byId('public.accounts')?.approved).toBe(true);
    expect(byId('public.accounts.id')?.approved).toBe(true);
    expect(byId('public.accounts.name')?.approved).toBe(true);

    const notes = byId('public.accounts.notes');
    expect(notes?.approved).toBe(false);
    expect(notes?.confidence).toBe('fuzzy');
    expect(mapping.needsAttention.some((m) => m.id === 'public.accounts.notes')).toBe(true);

    // The explicit needs_attention entry and the ignored item round-trip.
    expect(mapping.needsAttention.some((m) => m.id === 'public.audit_log')).toBe(true);
    expect(mapping.ignored).toHaveLength(1);
    expect(mapping.ignored[0].reason).toBe('not migrated');
    expect(mapping.rules).toHaveLength(1);
    expect(mapping.rules[0]).toContain('old_');
  });

  it('saveMapping exports the backend-built mapping', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'run_inventory') {
        return Promise.resolve(emptyInventory);
      }
      if (cmd === 'schema_diff') {
        return Promise.resolve(rawDiff());
      }
      if (cmd === 'build_mapping') {
        return Promise.resolve(rawMappingBuild());
      }
      if (cmd === 'export_mapping') {
        return Promise.resolve({ json: '{}', hash: 'abc123' });
      }
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const engine = createTauriEngine();
    const structure = await engine.compareStructure(profile(), profile());
    await engine.buildMapping(structure);
    const saved = await engine.saveMapping(mapping());

    expect(saved).toEqual({ json: '{}', hash: 'abc123' });
  });

  it('saveMapping requires buildMapping first', async () => {
    const engine = createTauriEngine();
    await expect(engine.saveMapping(mapping())).rejects.toThrow();
  });

  it('getVersion reads the engine version from the backend', async () => {
    invoke.mockResolvedValueOnce('0.1.0');
    const engine = createTauriEngine();
    await expect(engine.getVersion()).resolves.toBe('0.1.0');
    expect(invoke).toHaveBeenCalledWith('get_version');
  });
});

describe('level mapping (engine snake_case <-> contract)', () => {
  it('mapLevel maps the engine levels 1:1', () => {
    expect(mapLevel('structure')).toBe('structure');
    expect(mapLevel('counts_and_totals')).toBe('counts');
    expect(mapLevel('full_values')).toBe('full-values');
  });

  it('toEngineLevel reverses mapLevel', () => {
    expect(toEngineLevel('structure')).toBe('structure');
    expect(toEngineLevel('counts')).toBe('counts_and_totals');
    expect(toEngineLevel('full-values')).toBe('full_values');
  });
});

describe('progress-state mapping is strict', () => {
  it('maps known per-table states', () => {
    expect(mapTableState('done')).toBe('done');
    expect(mapTableState('running')).toBe('running');
    expect(mapTableState('paused')).toBe('paused');
    expect(mapTableState('cancelled')).toBe('cancelled');
    expect(mapTableState('failed')).toBe('failed');
    expect(mapTableState('changed-during-check')).toBe('changed-during-check');
  });

  it('treats an unknown per-table state as queued (never done)', () => {
    expect(mapTableState('bogus')).toBe('queued');
    expect(mapTableState('')).toBe('queued');
  });

  it('treats an unknown overall state as running (never done)', () => {
    expect(mapOverallState('bogus')).toBe('running');
    expect(mapOverallState('done')).toBe('done');
    expect(mapOverallState('cancelled')).toBe('cancelled');
    expect(mapOverallState('paused')).toBe('paused');
  });
});

describe('data half is wired to the backend', () => {
  it('exportReport is still NOT_AVAILABLE (T024)', async () => {
    const engine = createTauriEngine();
    const call = engine.exportReport({ format: 'pdf', includeKeys: false, filePath: '/tmp/x' });
    await expect(call).rejects.toBeInstanceOf(EngineError);
    await expect(call).rejects.toMatchObject({ code: 'NOT_AVAILABLE' });
  });

  it('buildCheckPlan invokes build_check_plan and maps the plan', async () => {
    const engine = createTauriEngine();
    // Satisfy the compare/buildMapping prerequisites.
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'run_inventory') return Promise.resolve(emptyInventory);
      if (cmd === 'schema_diff') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(rawMappingBuild());
      if (cmd === 'build_check_plan') {
        return Promise.resolve({
          tables: [
            {
              table_id: 'demo_bank.accounts',
              schema_name: 'demo_bank',
              table_name: 'accounts',
              level: 'full_values',
              control_total_columns: ['balance'],
            },
          ],
          presets: ['full values everywhere'],
          estimate_seconds_min: 2,
          estimate_seconds_max: 8,
        });
      }
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const structure = await engine.compareStructure(profile(), profile());
    await engine.buildMapping(structure);
    const plan = await engine.buildCheckPlan(mapping());

    expect(invoke).toHaveBeenCalledWith('build_check_plan', expect.objectContaining({}));
    expect(plan.tables).toHaveLength(1);
    expect(plan.tables[0].level).toBe('full-values');
    expect(plan.tables[0].controlTotalColumns).toEqual(['balance']);
    expect(plan.estimateSecondsMin).toBe(2);
  });

  it('startRun sends the resolved levels and control totals', async () => {
    const engine = createTauriEngine();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'run_inventory') return Promise.resolve(emptyInventory);
      if (cmd === 'schema_diff') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(rawMappingBuild());
      if (cmd === 'start_run') return Promise.resolve({ run_id: 'run-abc' });
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const structure = await engine.compareStructure(profile(), profile());
    await engine.buildMapping(structure);

    const started = await engine.startRun({
      oldProfile: defaultOldProfile(),
      newProfile: defaultOldProfile(),
      mapping: mapping(),
      plan: {
        tables: [
          {
            tableId: 't0',
            schemaName: 'demo_bank',
            tableName: 'accounts',
            level: 'counts',
            controlTotalColumns: ['balance'],
          },
        ],
        presets: [],
        estimateSecondsMin: 1,
        estimateSecondsMax: 2,
      },
    });

    expect(invoke).toHaveBeenCalledWith(
      'start_run',
      expect.objectContaining({
        input: expect.objectContaining({
          levels: { 'demo_bank.accounts': 'counts_and_totals' },
          control_total_columns: ['balance'],
          default_level: 'full_values',
        }),
      }),
    );
    expect(started.runId).toBe('run-abc');
    expect(started.overallState).toBe('running');
  });

  it('getResults maps the engine results and demotes an unknown verdict', async () => {
    const engine = createTauriEngine();
    invoke.mockResolvedValueOnce({
      run_id: 'run-1',
      verdict: 'FAIL',
      coverage: {
        total_tables: 2,
        full_values_tables: 1,
        counts_tables: 1,
        structure_tables: 0,
        ignored_tables: 0,
      },
      ignored: [{ id: 'demo_bank.legacy', name: 'demo_bank.legacy', reason: 'not migrated' }],
      failures: [
        {
          id: 'fail-demo_bank.accounts',
          table_id: 'demo_bank.accounts',
          schema_name: 'demo_bank',
          table_name: 'accounts',
          level: 'full_values',
          verdict: 'FAIL',
          summary: '12 rows missing',
          detail: 'new has fewer rows',
        },
      ],
      table_results: [
        {
          table_id: 'demo_bank.accounts',
          schema_name: 'demo_bank',
          table_name: 'accounts',
          verdict: 'FAIL',
          level: 'full_values',
          checked: true,
          drill_down: [
            { key: '12', kind: 'missing', differing_columns: null },
          ],
        },
        {
          table_id: 'demo_bank.other',
          schema_name: 'demo_bank',
          table_name: 'other',
          verdict: 'BOGUS_VERDICT',
          level: 'full_values',
          checked: true,
          drill_down: null,
        },
      ],
    });

    const results = await engine.getResults('run-1');

    expect(invoke).toHaveBeenCalledWith('get_results', { runId: 'run-1' });
    expect(results.verdict).toBe('FAIL');
    expect(results.coverage.totalTables).toBe(2);
    expect(results.failures).toHaveLength(1);
    expect(results.tableResults).toHaveLength(2);
    // Unknown verdict => TOOL_FAULT (non-PASS), never PASS.
    expect(results.tableResults[1].verdict).toBe('TOOL_FAULT');
    expect(results.tableResults[0].drillDown?.[0].kind).toBe('missing');
  });

  it('getResults maps every engine verdict 1:1 (never fabricates)', async () => {
    const engine = createTauriEngine();
    const rawResults = (verdict: string) => ({
      run_id: 'run-1',
      verdict,
      coverage: {
        total_tables: 1,
        full_values_tables: 1,
        counts_tables: 0,
        structure_tables: 0,
        ignored_tables: 0,
      },
      ignored: [],
      failures: [],
      table_results: [
        {
          table_id: 'demo_bank.t',
          schema_name: 'demo_bank',
          table_name: 't',
          verdict,
          level: 'full_values',
          checked: true,
          drill_down: null,
        },
      ],
    });

    for (const v of [
      'PASS',
      'FAIL',
      'WIDELY_DIFFERENT',
      'NEEDS_REVIEW',
      'CHANGED_DURING_CHECK',
      'TOOL_FAULT',
      'WARNING',
    ]) {
      invoke.mockReset();
      invoke.mockResolvedValueOnce(rawResults(v));
      const results = await engine.getResults('run-1');
      expect(results.verdict).toBe(v);
      expect(results.tableResults[0].verdict).toBe(v);
    }
  });

  it('pollRun / setRunPaused / cancelRun invoke their commands', async () => {
    const engine = createTauriEngine();
    const progress = {
      run_id: 'run-1',
      overall_state: 'running',
      tables: [
        {
          table_id: 'demo_bank.accounts',
          schema_name: 'demo_bank',
          table_name: 'accounts',
          state: 'done',
          progress_percent: 100,
          eta_seconds: null,
        },
      ],
      eta_seconds_min: null,
      eta_seconds_max: null,
    };
    invoke.mockResolvedValue(progress);

    await expect(engine.pollRun('run-1')).resolves.toMatchObject({ runId: 'run-1' });
    expect(invoke).toHaveBeenCalledWith('poll_run', { runId: 'run-1' });

    await engine.setRunPaused('run-1', true);
    expect(invoke).toHaveBeenCalledWith('set_run_paused', { runId: 'run-1', paused: true });

    await engine.cancelRun('run-1');
    expect(invoke).toHaveBeenCalledWith('cancel_run', { runId: 'run-1' });
  });
});

function mapping(): Mapping {
  return { rules: [], matches: [], needsAttention: [], ignored: [] };
}

function inventory(): Inventory {
  return {
    schemas: [],
    oldEncoding: 'UTF8',
    newEncoding: 'UTF8',
    oldCollation: 'C',
    newCollation: 'C',
  };
}
