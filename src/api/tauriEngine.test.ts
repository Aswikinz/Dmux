import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createTauriEngine,
  mapDiffStatus,
  mapLevel,
  mapOverallState,
  mapTableState,
  mapVerdict,
  RunFailedError,
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
      allowSuperuser: null,
    });
    expect(result.ok).toBe(true);
    expect(result.info?.version).toBe('17.0');
    expect(result.info?.tls).toEqual({ state: 'encrypted', verified: true });
  });

  it('testConnection reports a session-only password, and never a password value', async () => {
    invoke.mockResolvedValueOnce({ ...rawServerInfo, session_password: true });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.info?.sessionPassword).toBe(true);
    invoke.mockResolvedValueOnce({ ...rawServerInfo });
    const plain = await engine.testConnection(profile());
    expect(plain.info?.sessionPassword).toBe(false);
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
      if (cmd === 'compare_structure') {
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

  it('compareStructure sends only connection details, never an inventory', async () => {
    // Inventories hold 64-bit sequence bounds (up to i64::MAX) that a JS
    // number rounds to 9223372036854776000; they must stay in the engine.
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const engine = createTauriEngine();
    await engine.compareStructure(profile({ port: 5433 }), profile({ port: 5434 }));

    expect(invoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = invoke.mock.calls[0] as [string, Record<string, unknown>];
    expect(cmd).toBe('compare_structure');
    expect(Object.keys(args).sort()).toEqual(['allowSuperuser', 'new', 'old']);
    expect(args.allowSuperuser).toBeNull();
    const oldArg = args.old as { profile: { port: number } };
    const newArg = args.new as { profile: { port: number } };
    expect(oldArg.profile.port).toBe(5433);
    expect(newArg.profile.port).toBe(5434);
  });

  it('buildMapping requires compareStructure first', async () => {
    const engine = createTauriEngine();
    await expect(engine.buildMapping(inventory())).rejects.toThrow();
  });

  it('buildMapping only auto-approves exact and case_insensitive matches', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') {
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
      if (cmd === 'compare_structure') {
        return Promise.resolve(rawDiff());
      }
      if (cmd === 'build_mapping') {
        return Promise.resolve(rawMappingBuild());
      }
      if (cmd === 'save_mapping') {
        return Promise.resolve({ status: 'saved', path: '/home/u/m.json', hash: 'abc123' });
      }
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const engine = createTauriEngine();
    const structure = await engine.compareStructure(profile(), profile());
    await engine.buildMapping(structure);
    const saved = await engine.saveMapping(mapping());

    expect(saved).toEqual({ ok: true, filePath: '/home/u/m.json', hash: 'abc123' });
    expect(invoke).toHaveBeenLastCalledWith(
      'save_mapping',
      expect.objectContaining({ mapping: expect.objectContaining({ version: expect.any(Number) }) }),
    );
  });

  it('saveMapping says cancelled, or the error in plain words', async () => {
    let answer: unknown = { status: 'cancelled' };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(rawMappingBuild());
      if (cmd === 'save_mapping') {
        return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer);
      }
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });
    const engine = createTauriEngine();
    await engine.buildMapping(await engine.compareStructure(profile(), profile()));
    expect(await engine.saveMapping(mapping())).toEqual({ ok: false, cancelled: true });
    answer = new Error('the mapping could not be written: disk full');
    const failed = await engine.saveMapping(mapping());
    expect(failed.ok).toBe(false);
    expect(failed.error).toMatch(/disk full/);
  });

  it('buildMapping says which table each pair belongs to, with bare column names', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(rawMappingBuild());
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });
    const engine = createTauriEngine();
    const built = await engine.buildMapping(await engine.compareStructure(profile(), profile()));
    const table = built.matches.find((m) => m.id === 'public.accounts');
    expect(table).toMatchObject({ kind: 'table', group: 'public.accounts' });
    const renamed = built.matches.find((m) => m.id === 'public.accounts.name');
    expect(renamed).toMatchObject({
      kind: 'column',
      group: 'public.accounts',
      oldLabel: 'name',
      newLabel: 'full_name',
    });
    // The display fields never touch what the queue acts on.
    expect(renamed?.ref).toBeUndefined();
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

describe('profile commands', () => {
  const rawStoredProfile = {
    id: 'old-profile',
    label: 'Old database',
    host: '127.0.0.1',
    port: 5433,
    database: 'bank',
    user: 'dmux_dev',
    encryption: 'verify',
    tls_fingerprint: null,
    plaintext_reason: null,
  };

  it('saveProfile sends the profile and a typed password to save_profile', async () => {
    invoke.mockResolvedValueOnce({ profile: rawStoredProfile, warning: null });
    const engine = createTauriEngine();
    const result = await engine.saveProfile(profile(), 'hunter2');

    expect(invoke).toHaveBeenCalledWith('save_profile', {
      profile: expect.objectContaining({ encryption: 'verify' }),
      password: 'hunter2',
    });
    expect(result.warning).toBeUndefined();
    expect(result.profile.host).toBe('127.0.0.1');
  });

  it('saveProfile maps a keychain-unavailable warning back to the contract', async () => {
    invoke.mockResolvedValueOnce({
      profile: rawStoredProfile,
      warning: 'password not saved; you will be asked each run',
    });
    const engine = createTauriEngine();
    const result = await engine.saveProfile(profile(), 'hunter2');
    expect(result.warning).toContain('asked each run');
  });

  it('saveProfile sends null when no password is typed', async () => {
    invoke.mockResolvedValueOnce({ profile: rawStoredProfile, warning: null });
    const engine = createTauriEngine();
    await engine.saveProfile(profile());
    expect(invoke).toHaveBeenCalledWith(
      'save_profile',
      expect.objectContaining({ password: null }),
    );
  });

  it('profileHasPassword invokes profile_has_password', async () => {
    invoke.mockResolvedValueOnce(true);
    const engine = createTauriEngine();
    await expect(engine.profileHasPassword('old-profile')).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith('profile_has_password', { id: 'old-profile' });
  });

  it('testConnection sends a typed one-time password through the connect input', async () => {
    invoke.mockResolvedValueOnce(rawServerInfo);
    const engine = createTauriEngine();
    await engine.testConnection(profile(), 'hunter2');
    expect(invoke).toHaveBeenCalledWith('probe_connection', {
      input: expect.objectContaining({ password: 'hunter2' }),
      allowSuperuser: null,
    });
  });

  it('a superuser is refused until the user allows it with a reason, which every call then carries', async () => {
    const engine = createTauriEngine();
    invoke.mockRejectedValueOnce({ kind: 'superuser_refused' });
    const refused = await engine.testConnection(profile());
    expect(refused.ok).toBe(false);
    expect(refused.superuserRefused).toBe(true);

    // A blank reason is not an override.
    engine.setSuperuserOverride({ reason: '   ' });
    invoke.mockResolvedValueOnce({ ...rawServerInfo, is_superuser: true });
    await engine.testConnection(profile());
    expect(invoke).toHaveBeenLastCalledWith(
      'probe_connection',
      expect.objectContaining({ allowSuperuser: null }),
    );

    engine.setSuperuserOverride({ reason: ' migration rehearsal on a copy ' });
    invoke.mockResolvedValueOnce({ ...rawServerInfo, is_superuser: true });
    const ok = await engine.testConnection(profile());
    expect(ok.ok).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith(
      'probe_connection',
      expect.objectContaining({
        allowSuperuser: { allow: true, reason: 'migration rehearsal on a copy' },
      }),
    );

    // Structure and the run carry it too.
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(rawMappingBuild());
      return Promise.resolve({});
    });
    await engine.compareStructure(profile(), profile());
    expect(invoke).toHaveBeenCalledWith(
      'compare_structure',
      expect.objectContaining({
        allowSuperuser: { allow: true, reason: 'migration rehearsal on a copy' },
      }),
    );

    // Revoking it goes back to refusing.
    engine.setSuperuserOverride(null);
    invoke.mockResolvedValueOnce({ ...rawServerInfo });
    await engine.testConnection(profile());
    expect(invoke).toHaveBeenLastCalledWith(
      'probe_connection',
      expect.objectContaining({ allowSuperuser: null }),
    );
  });

  it('testConnection surfaces the ask-for-password prompt on password_missing', async () => {
    invoke.mockRejectedValueOnce({ kind: 'password_missing', reason: {}, profile: 'old-profile' });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());
    expect(result.ok).toBe(false);
    expect(result.error).toContain('type the password');
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

describe('a failed run', () => {
  it('surfaces the typed error from every progress call, never "running"', async () => {
    const engine = createTauriEngine();
    const failed = {
      run_id: 'run-x',
      overall_state: 'failed',
      tables: [],
      eta_seconds_min: null,
      eta_seconds_max: null,
      error: { kind: 'database', message: 'database "nope" does not exist' },
    };
    invoke.mockImplementation(() => Promise.resolve(failed));
    for (const call of [
      () => engine.pollRun('run-x'),
      () => engine.setRunPaused('run-x', true),
      () => engine.cancelRun('run-x'),
    ]) {
      const p = call();
      await expect(p).rejects.toBeInstanceOf(RunFailedError);
      await expect(p).rejects.toMatchObject({ kind: 'database' });
      await expect(p).rejects.toThrow(/does not exist/);
    }
  });
});

describe('data half is wired to the backend', () => {
  it('exportReport sends only the run id, format and key choice, and maps a saved path', async () => {
    invoke.mockResolvedValueOnce({ status: 'saved', path: '/home/u/dmux-run-1.pdf' });
    const engine = createTauriEngine();
    const res = await engine.exportReport({ runId: 'run-1', format: 'pdf', includeKeys: false });
    expect(invoke).toHaveBeenCalledWith('export_report', {
      input: { run_id: 'run-1', format: 'pdf', include_keys: false },
    });
    expect(res).toEqual({ ok: true, filePath: '/home/u/dmux-run-1.pdf' });
  });

  it('exportReport reports a dismissed dialog as cancelled', async () => {
    invoke.mockResolvedValueOnce({ status: 'cancelled' });
    const engine = createTauriEngine();
    const res = await engine.exportReport({ runId: 'run-1', format: 'excel', includeKeys: true });
    expect(invoke).toHaveBeenCalledWith('export_report', {
      input: { run_id: 'run-1', format: 'excel', include_keys: true },
    });
    expect(res).toEqual({ ok: false, cancelled: true });
  });

  it('exportReport turns a typed backend error into plain text', async () => {
    invoke.mockRejectedValueOnce({
      kind: 'results_not_available',
      message: 'results are not available: the run has not finished',
    });
    const engine = createTauriEngine();
    const res = await engine.exportReport({ runId: 'run-1', format: 'pdf', includeKeys: false });
    expect(res.ok).toBe(false);
    expect(res.error).toContain('has not finished');
  });

  it('buildCheckPlan invokes build_check_plan and maps the plan', async () => {
    const engine = createTauriEngine();
    // Satisfy the compare/buildMapping prerequisites.
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
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
              size_bytes: 5242880,
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
    // What the backend lists is a suggestion; nothing is confirmed yet.
    expect(plan.tables[0].controlTotalSuggestions).toEqual(['balance']);
    expect(plan.tables[0].controlTotalColumns).toBeUndefined();
    expect(plan.tables[0].sizeBytes).toBe(5242880);
    expect(plan.estimateSecondsMin).toBe(2);
  });

  it('startRun sends the resolved levels and control totals', async () => {
    const engine = createTauriEngine();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
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

  it('startRun leaves out the tables the user did not choose, and gives them no level', async () => {
    const engine = createTauriEngine();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(rawMappingBuild());
      if (cmd === 'start_run') return Promise.resolve({ run_id: 'run-s' });
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });
    const structure = await engine.compareStructure(profile(), profile());
    await engine.buildMapping(structure);
    const t = (name: string, included?: boolean) => ({
      tableId: name,
      schemaName: 'demo_bank',
      tableName: name,
      level: 'full-values' as const,
      included,
    });
    await engine.startRun({
      oldProfile: defaultOldProfile(),
      newProfile: defaultOldProfile(),
      mapping: mapping(),
      plan: {
        tables: [t('accounts', true), t('loans', false), t('ledger')],
        presets: [],
        estimateSecondsMin: 1,
        estimateSecondsMax: 2,
      },
    });
    expect(invoke).toHaveBeenCalledWith(
      'start_run',
      expect.objectContaining({
        input: expect.objectContaining({
          levels: { 'demo_bank.accounts': 'full_values', 'demo_bank.ledger': 'full_values' },
          skip_tables: ['demo_bank.loans'],
        }),
      }),
    );
  });

  it('startRun passes the run window, and none when it is off', async () => {
    const engine = createTauriEngine();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(rawMappingBuild());
      if (cmd === 'start_run') return Promise.resolve({ run_id: 'run-w' });
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });
    const structure = await engine.compareStructure(profile(), profile());
    await engine.buildMapping(structure);
    const plan = { tables: [], presets: [], estimateSecondsMin: 1, estimateSecondsMax: 2 };
    const def = { oldProfile: defaultOldProfile(), newProfile: defaultOldProfile(), mapping: mapping() };
    await engine.startRun({ ...def, plan: { ...plan, runWindow: '22:00-05:00' } });
    expect(invoke).toHaveBeenCalledWith(
      'start_run',
      expect.objectContaining({ input: expect.objectContaining({ run_window: '22:00-05:00' }) }),
    );
    await engine.startRun({ ...def, plan });
    expect(invoke).toHaveBeenLastCalledWith(
      'start_run',
      expect.objectContaining({ input: expect.objectContaining({ run_window: null }) }),
    );
  });

  it('pollRun maps the window wait, the notices and the time remaining', async () => {
    const engine = createTauriEngine();
    invoke.mockResolvedValueOnce({
      run_id: 'r',
      overall_state: 'running',
      tables: [
        { table_id: 'a.b', schema_name: 'a', table_name: 'b', state: 'running', progress_percent: 0, eta_seconds: 30 },
      ],
      eta_seconds_min: 10,
      eta_seconds_max: 90,
      waiting_for_window: '22:00',
      notices: ['note'],
    });
    const p = await engine.pollRun('r');
    expect(p.waitingForWindow).toBe('22:00');
    expect(p.notices).toEqual(['note']);
    expect([p.etaSecondsMin, p.etaSecondsMax, p.tables[0].etaSeconds]).toEqual([10, 90, 30]);
    invoke.mockResolvedValueOnce({
      run_id: 'r',
      overall_state: 'running',
      tables: [],
      eta_seconds_min: null,
      eta_seconds_max: null,
      waiting_for_window: null,
      notices: [],
    });
    const q = await engine.pollRun('r');
    expect(q.waitingForWindow).toBeUndefined();
    expect(q.notices).toBeUndefined();
    expect(q.etaSecondsMin).toBeUndefined();
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

  it('getResults maps options_applied and per-column notes', async () => {
    const engine = createTauriEngine();
    invoke.mockResolvedValueOnce({
      run_id: 'run-1',
      verdict: 'PASS',
      options_applied: true,
      options_recorded: true,
      warnings: ['the old database login can write'],
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
          table_id: 'demo_bank.accounts',
          schema_name: 'demo_bank',
          table_name: 'accounts',
          verdict: 'PASS',
          level: 'full_values',
          checked: true,
          drill_down: null,
          column_notes: [{ column: 'balance', kind: 'option', note: 'round to 2 decimals' }],
        },
      ],
    });

    const results = await engine.getResults('run-1');
    expect(results.optionsApplied).toBe(true);
    expect(results.optionsRecorded).toBe(true);
    expect(results.warnings).toEqual(['the old database login can write']);
    expect(results.tableResults[0].columnNotes).toEqual([
      { column: 'balance', kind: 'option', note: 'round to 2 decimals' },
    ]);
  });

  it('getResults defaults options_recorded to false when the backend omits it', async () => {
    // A v2-era backend (or results file) never records options: the adapter must
    // say "options not recorded", never assume none were applied (never a bare
    // PASS).  The field is absent here on purpose.
    const engine = createTauriEngine();
    invoke.mockResolvedValueOnce({
      run_id: 'run-v2',
      verdict: 'PASS',
      coverage: {
        total_tables: 1,
        full_values_tables: 1,
        counts_tables: 0,
        structure_tables: 0,
        ignored_tables: 0,
      },
      ignored: [],
      failures: [],
      table_results: [],
    });

    const results = await engine.getResults('run-v2');
    expect(results.optionsRecorded).toBe(false);
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

describe('profiles and history are wired to the backend', () => {
  it('listProfiles and deleteProfile call the backend and keep the CA file', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'p1',
        label: 'Prod',
        host: 'h',
        port: 5432,
        database: 'd',
        user: 'u',
        encryption: 'verify',
        tls_fingerprint: null,
        plaintext_reason: null,
        ca_file: '/etc/ca.pem',
      },
    ]);
    const engine = createTauriEngine();
    const list = await engine.listProfiles();
    expect(invoke).toHaveBeenCalledWith('list_profiles');
    expect(list).toEqual([
      expect.objectContaining({ id: 'p1', label: 'Prod', caFile: '/etc/ca.pem' }),
    ]);
    invoke.mockResolvedValueOnce(true);
    expect(await engine.deleteProfile('p1')).toBe(true);
    expect(invoke).toHaveBeenCalledWith('delete_profile', { id: 'p1' });
  });

  it('listHistory maps rows, turning seconds into ISO dates', async () => {
    invoke.mockResolvedValueOnce([
      {
        id: 'run-1',
        started_at: 1_700_000_000,
        ended_at: 1_700_000_600,
        verdict: 'FAIL',
        options_applied: false,
        options_recorded: true,
        coverage_line: '1 of 1 tables fully checked',
        old_label: 'Old',
        new_label: 'New',
        mapping_hash: 'mh',
        definition_hash: 'dh',
        results_sha256: 'sha',
      },
      { id: 'run-2', started_at: 0, ended_at: 0, verdict: 'something odd', options_applied: false,
        options_recorded: false, coverage_line: '', old_label: '', new_label: '',
        mapping_hash: '', definition_hash: '', results_sha256: '' },
    ]);
    const engine = createTauriEngine();
    const rows = await engine.listHistory();
    expect(invoke).toHaveBeenCalledWith('list_history');
    expect(rows[0]).toMatchObject({
      id: 'run-1',
      startedAt: '2023-11-14T22:13:20.000Z',
      verdict: 'FAIL',
      coverage: '1 of 1 tables fully checked',
      resultsSha256: 'sha',
    });
    // An unknown verdict is never PASS.
    expect(rows[1].verdict).toBe('TOOL_FAULT');
  });

  const rawResults = {
    run_id: 'run-opened',
    verdict: 'FAIL',
    coverage: {
      total_tables: 1,
      full_values_tables: 1,
      counts_tables: 0,
      structure_tables: 0,
      ignored_tables: 0,
    },
    ignored: [],
    failures: [],
    table_results: [],
    options_applied: false,
    options_recorded: true,
    warnings: [],
  };

  it('openHistoryEntry sends only the id and maps the verified results', async () => {
    invoke.mockResolvedValueOnce(rawResults);
    const results = await createTauriEngine().openHistoryEntry('run-1');
    expect(invoke).toHaveBeenCalledWith('open_history_entry', { id: 'run-1' });
    expect(results.runId).toBe('run-opened');
    expect(results.verdict).toBe('FAIL');
  });

  it('openResultsFile maps opened, cancelled and refused outcomes, with no path sent', async () => {
    const engine = createTauriEngine();
    invoke.mockResolvedValueOnce({ status: 'opened', results: rawResults });
    const opened = await engine.openResultsFile();
    expect(invoke).toHaveBeenCalledWith('open_results_file');
    expect(opened.ok).toBe(true);
    expect(opened.results?.runId).toBe('run-opened');

    invoke.mockResolvedValueOnce({ status: 'cancelled' });
    expect(await engine.openResultsFile()).toEqual({ ok: false, cancelled: true });

    invoke.mockRejectedValueOnce({ kind: 'report', message: 'changed since Dmux wrote it' });
    const refused = await engine.openResultsFile();
    expect(refused.ok).toBe(false);
    expect(refused.results).toBeUndefined();
    expect(refused.error).toContain('changed since Dmux wrote it');
  });

  it('history log, save log and purge call the backend', async () => {
    const engine = createTauriEngine();
    invoke.mockResolvedValueOnce('Dmux run log\n');
    expect(await engine.getHistoryLog('run-1')).toBe('Dmux run log\n');
    expect(invoke).toHaveBeenCalledWith('get_history_log', { id: 'run-1' });

    invoke.mockResolvedValueOnce({ status: 'saved', path: '/home/u/log.txt' });
    expect(await engine.saveHistoryLog('run-1')).toEqual({ ok: true, filePath: '/home/u/log.txt' });
    expect(invoke).toHaveBeenCalledWith('save_history_log', { id: 'run-1' });
    invoke.mockResolvedValueOnce({ status: 'cancelled' });
    expect(await engine.saveHistoryLog('run-1')).toEqual({ ok: false, cancelled: true });
    invoke.mockRejectedValueOnce({ kind: 'report', message: 'could not be written' });
    expect((await engine.saveHistoryLog('run-1')).error).toContain('could not be written');

    invoke.mockResolvedValueOnce(3);
    expect(await engine.purgeHistory()).toBe(3);
    expect(invoke).toHaveBeenCalledWith('purge_history');
  });
});

describe('mapping suggestions, edits and rules', () => {
  /** A build with one unmatched table (with a suggestion) and one new-only table. */
  function editableBuild() {
    return {
      mapping: {
        version: 1,
        rules: [{ kind: 'prefix_replace', pattern: 'tbl_', target: '' }],
        schema_mappings: {
          public: {
            target_schema: 'public',
            table_mappings: {
              notes: { target_table: 'notes', status: 'exact', column_mappings: {} },
            },
          },
        },
        ignored: [],
        mapping_hash: 'h1',
      },
      needs_attention: [
        { path: 'public.acct', schema: 'public', table: 'acct', column: null, issue: 'missing_in_new', candidates: [] },
        { path: 'public.accounts', schema: 'public', table: 'accounts', column: null, issue: 'add_back_review', candidates: [] },
        { path: 'public.audit', schema: 'public', table: 'audit', column: null, issue: 'add_back_review', candidates: [] },
      ],
      suggestions: [
        { path: 'public.acct', schema: 'public', table: 'acct', column: null, candidate: 'accounts', confidence: 71 },
      ],
    };
  }

  async function ready() {
    const engine = createTauriEngine();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'compare_structure') return Promise.resolve(rawDiff());
      if (cmd === 'build_mapping') return Promise.resolve(editableBuild());
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });
    const structure = await engine.compareStructure(profile(), profile());
    const built = await engine.buildMapping(structure);
    return { engine, built };
  }

  it('shows suggestions with a confidence, an unmatched item with no new name, and the new-only items', async () => {
    const { built } = await ready();
    expect(built.suggestions).toEqual([
      {
        id: 'public.acct',
        ref: { schema: 'public', table: 'acct' },
        candidate: 'accounts',
        confidence: 71,
      },
    ]);
    const item = built.needsAttention.find((m) => m.id === 'public.acct');
    expect(item?.newName).toBe('');
    expect(item?.ref).toEqual({ schema: 'public', table: 'acct' });
    // New-only tables are not decisions; they feed the pairing picker.
    expect(built.needsAttention.some((m) => m.id === 'public.accounts')).toBe(false);
    expect(built.unmatchedNew?.map((u) => u.ref.table)).toEqual(['accounts', 'audit']);
    expect(built.ruleSpecs).toEqual([{ kind: 'prefix_replace', pattern: 'tbl_', target: '' }]);
  });

  it('accepting a suggestion goes through edit_mapping and changes what saveMapping and startRun send', async () => {
    const { engine, built } = await ready();
    const edited = editableBuild().mapping;
    edited.mapping_hash = 'h2';
    (edited.schema_mappings.public.table_mappings as Record<string, unknown>).acct = {
      target_table: 'accounts',
      status: 'confirmed',
      column_mappings: {},
    };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'edit_mapping') return Promise.resolve(edited);
      if (cmd === 'save_mapping') return Promise.resolve({ status: 'saved', path: '/m.json', hash: 'h2' });
      if (cmd === 'start_run') return Promise.resolve({ run_id: 'run-x' });
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const next = await engine.editMapping(built, [
      { kind: 'pair_table', schema: 'public', table: 'acct', newTable: 'accounts' },
    ]);

    // The wire form carries separate names, and the backend's own mapping, never the webview's.
    const call = invoke.mock.calls.find((c) => c[0] === 'edit_mapping');
    expect(call?.[1]).toMatchObject({
      edits: [{ kind: 'pair_table', schema: 'public', table: 'acct', new_table: 'accounts' }],
      mapping: expect.objectContaining({ mapping_hash: 'h1' }),
    });
    // The pairing is now a confirmed match; the item, its suggestion and the
    // new table it took have left the lists.
    const pair = next.matches.find((m) => m.id === 'public.acct');
    expect(pair).toMatchObject({ newName: 'public.accounts', confidence: 'confirmed', approved: true });
    expect(next.needsAttention.some((m) => m.id === 'public.acct')).toBe(false);
    expect(next.suggestions).toEqual([]);
    expect(next.unmatchedNew?.map((u) => u.ref.table)).toEqual(['audit']);

    await engine.saveMapping(next);
    expect(invoke).toHaveBeenCalledWith(
      'save_mapping',
      expect.objectContaining({ mapping: expect.objectContaining({ mapping_hash: 'h2' }) }),
    );
    await engine.startRun({
      oldProfile: defaultOldProfile(),
      newProfile: defaultOldProfile(),
      mapping: next,
      plan: { tables: [], presets: [], estimateSecondsMin: 0, estimateSecondsMax: 0 },
    });
    expect(invoke).toHaveBeenCalledWith(
      'start_run',
      expect.objectContaining({
        input: expect.objectContaining({
          mapping: expect.objectContaining({ mapping_hash: 'h2' }),
        }),
      }),
    );
  });

  it('a refused edit is thrown as its plain message and leaves the mapping as it was', async () => {
    const { engine, built } = await ready();
    invoke.mockImplementation((cmd: string) =>
      cmd === 'edit_mapping'
        ? Promise.reject({
            kind: 'invalid_mapping',
            message: 'public.a is already paired with public.accounts. Two old tables cannot map to one new table: unpair one of them first.',
          })
        : Promise.reject(new Error(`unexpected command ${cmd}`)),
    );
    await expect(
      engine.editMapping(built, [
        { kind: 'pair_table', schema: 'public', table: 'acct', newTable: 'accounts' },
      ]),
    ).rejects.toThrow(/Two old tables cannot map to one new table/);
    // The held mapping is unchanged: saving still sends the original.
    invoke.mockImplementation(() => Promise.resolve({ json: '{}', hash: 'h1' }));
    await engine.saveMapping(built);
    expect(invoke).toHaveBeenLastCalledWith(
      'save_mapping',
      expect.objectContaining({ mapping: expect.objectContaining({ mapping_hash: 'h1' }) }),
    );
  });

  it('previews rules without changing the mapping, and rebuilds with them', async () => {
    const { engine } = await ready();
    const rules = [{ kind: 'prefix_replace' as const, pattern: 'a_', target: 'b_' }];
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'preview_rules')
        return Promise.resolve({ tables_total: 10, tables_by_rules: 4, tables_matched: 9 });
      if (cmd === 'rebuild_mapping') {
        const b = editableBuild();
        b.mapping.rules = [{ kind: 'prefix_replace', pattern: 'a_', target: 'b_' }];
        b.mapping.mapping_hash = 'h3';
        return Promise.resolve(b);
      }
      if (cmd === 'save_mapping') return Promise.resolve({ status: 'saved', path: '/m.json', hash: 'h3' });
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });
    await expect(engine.previewRules(rules)).resolves.toEqual({
      tablesTotal: 10,
      tablesByRules: 4,
      tablesMatched: 9,
    });
    expect(invoke).toHaveBeenCalledWith('preview_rules', expect.objectContaining({ rules }));
    // A preview alone changes nothing the run would use.
    await engine.saveMapping(mapping());
    expect(invoke).toHaveBeenLastCalledWith(
      'save_mapping',
      expect.objectContaining({ mapping: expect.objectContaining({ mapping_hash: 'h1' }) }),
    );

    const rebuilt = await engine.rebuildMapping(rules);
    expect(rebuilt.ruleSpecs).toEqual(rules);
    await engine.saveMapping(rebuilt);
    expect(invoke).toHaveBeenLastCalledWith(
      'save_mapping',
      expect.objectContaining({ mapping: expect.objectContaining({ mapping_hash: 'h3' }) }),
    );
  });

  it('only confirmed control-total columns are sent to start_run', async () => {
    const { engine, built } = await ready();
    invoke.mockImplementation((cmd: string) =>
      cmd === 'start_run' ? Promise.resolve({ run_id: 'r' }) : Promise.reject(new Error(cmd)),
    );
    const table = (name: string, confirmed?: string[]) => ({
      tableId: `public.${name}`,
      schemaName: 'public',
      tableName: name,
      level: 'full-values' as const,
      controlTotalSuggestions: ['amount', 'balance'],
      controlTotalColumns: confirmed,
    });
    await engine.startRun({
      oldProfile: defaultOldProfile(),
      newProfile: defaultOldProfile(),
      mapping: built,
      plan: {
        tables: [table('a'), table('b', ['balance'])],
        presets: [],
        estimateSecondsMin: 0,
        estimateSecondsMax: 0,
      },
    });
    expect(invoke).toHaveBeenCalledWith(
      'start_run',
      expect.objectContaining({
        input: expect.objectContaining({ control_total_columns: ['balance'] }),
      }),
    );
  });
});

describe('error text', () => {
  it('never shows [object Object] and explains the typed backend errors', async () => {
    const { describeError } = await import('./tauriEngine');
    expect(describeError({ kind: 'password_missing' })).toContain('no password is stored');
    expect(describeError({ kind: 'password_remove_failed' })).toContain('keeps the password in memory');
    expect(describeError({ kind: 'database', message: 'connection refused' })).toBe('connection refused');
    expect(describeError(new Error('boom'))).toBe('boom');
    expect(describeError('plain')).toBe('plain');
    for (const odd of [{}, { foo: 1 }, null, undefined, 42]) {
      const text = describeError(odd);
      expect(text).not.toContain('[object Object]');
      expect(text.length).toBeGreaterThan(0);
    }
  });
});

