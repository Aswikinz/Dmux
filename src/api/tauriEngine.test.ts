import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockEngine } from './mockEngine';
import { createTauriEngine, mapDiffStatus, mapVerdict } from './tauriEngine';
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
              },
            },
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
  it('testConnection surfaces server info on success', async () => {
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

  it('testConnection flags a superuser with a warning', async () => {
    invoke.mockResolvedValueOnce({ ...rawServerInfo, is_superuser: true });
    const engine = createTauriEngine();
    const result = await engine.testConnection(profile());

    expect(result.ok).toBe(true);
    expect(result.info?.isSuperuser).toBe(true);
    expect(result.info?.warning).toContain('superuser');
  });

  it('testConnection maps trust and plaintext encryption to the backend', async () => {
    invoke.mockResolvedValue(rawServerInfo);
    const engine = createTauriEngine();
    await engine.testConnection(profile({ encryption: 'allow-plaintext' }));

    expect(invoke).toHaveBeenCalledWith(
      'probe_connection',
      expect.objectContaining({
        input: expect.objectContaining({
          profile: expect.objectContaining({ encryption: 'allow-plaintext' }),
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
              {
                path: ['database', 'encoding'],
                status: 'warning',
                old_value: 'LATIN1',
                new_value: 'UTF8',
                message: 'encoding changed: LATIN1 -> UTF8',
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
    expect(inventory.schemas).toHaveLength(3);
    const missing = inventory.schemas.find((n) => n.name === 'archived');
    expect(missing?.status).toBe('missing');
    const changed = inventory.schemas.find((n) => n.name === 'accounts');
    expect(changed?.status).toBe('changed');
    const encoding = inventory.schemas.find((n) => n.name === 'encoding');
    expect(encoding?.status).toBe('changed');
    expect(encoding?.old).toBe('LATIN1');
    expect(encoding?.new).toBe('UTF8');
  });

  it('buildMapping requires compareStructure first', async () => {
    const engine = createTauriEngine();
    await expect(engine.buildMapping(inventory())).rejects.toThrow();
  });

  it('buildMapping translates the backend mapping to the contract', async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'run_inventory') {
        return Promise.resolve(emptyInventory);
      }
      if (cmd === 'schema_diff') {
        return Promise.resolve(rawDiff({ old_encoding: 'LATIN1', new_encoding: 'UTF8' }));
      }
      if (cmd === 'build_mapping') {
        return Promise.resolve(rawMappingBuild());
      }
      return Promise.reject(new Error(`unexpected command ${cmd}`));
    });

    const engine = createTauriEngine();
    const structure = await engine.compareStructure(profile(), profile());
    const mapping = await engine.buildMapping(structure);

    // A table match plus its two column matches.
    expect(mapping.matches).toHaveLength(3);
    expect(mapping.matches[0].confidence).toBe('exact');
    expect(mapping.matches[1].confidence).toBe('exact');
    expect(mapping.matches[2].confidence).toBe('case');
    // One flagged table + the encoding review item.
    expect(mapping.needsAttention).toHaveLength(2);
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
    await engine.compareStructure(profile(), profile());
    await engine.buildMapping(inventory());
    const saved = await engine.saveMapping(mapping());

    expect(saved).toEqual({ json: '{}', hash: 'abc123' });
  });

  it('saveMapping requires buildMapping first', async () => {
    const engine = createTauriEngine();
    await expect(engine.saveMapping(mapping())).rejects.toThrow();
  });

  it('delegates the data-half methods to the mock', async () => {
    const dataMock = createMockEngine();
    const spy = vi.spyOn(dataMock, 'buildCheckPlan');
    const engine = createTauriEngine({ dataMock });

    await engine.buildCheckPlan(mapping());
    expect(spy).toHaveBeenCalled();
  });

  it('getVersion reads the engine version from the backend', async () => {
    invoke.mockResolvedValueOnce('0.1.0');
    const engine = createTauriEngine();
    await expect(engine.getVersion()).resolves.toBe('0.1.0');
    expect(invoke).toHaveBeenCalledWith('get_version');
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