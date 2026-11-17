import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createTauriEngine, translateStructureRow } from './tauriEngine';
import { defaultOldProfile } from '../screens/ConnectScreen';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

beforeEach(() => {
  invoke.mockReset();
});

const coverage = {
  total_tables: 1,
  full_values_tables: 1,
  counts_tables: 0,
  structure_tables: 0,
  ignored_tables: 0,
};

const conn = (over: Record<string, unknown> = {}) => ({
  tls: 'verified',
  plaintext_reason: null,
  superuser: false,
  superuser_reason: null,
  can_write: false,
  server_version: '17.2',
  server_encoding: 'UTF8',
  ...over,
});

function rawResults(table: Record<string, unknown>, extra: Record<string, unknown> = {}) {
  return {
    run_id: 'run-1',
    verdict: 'PASS',
    coverage,
    ignored: [],
    failures: [],
    table_results: [
      {
        table_id: 'bank.accounts',
        schema_name: 'bank',
        table_name: 'accounts',
        verdict: 'PASS',
        level: 'full_values',
        checked: true,
        drill_down: null,
        column_notes: [],
        ...table,
      },
    ],
    options_applied: false,
    options_recorded: true,
    warnings: [],
    ...extra,
  };
}

describe('getResults maps the recorded facts', () => {
  it('carries counts, buckets, spot check, change detection, totals and rules', async () => {
    invoke.mockResolvedValueOnce(
      rawResults(
        {
          old_rows: 1200,
          new_rows: 1200,
          differing_buckets: 0,
          missing_buckets: 0,
          extra_buckets: 0,
          reasons: ['spot check: 200 rows compared value by value, 0 differ'],
          facts: {
            key_columns: ['id'],
            columns: [{ column: 'id', rule: 'compared as an exact whole number' }],
            buckets_total: 256,
            spot: { state: 'agreed', compared: 200, differing: 0, note: '' },
            change_detection_old: 'available',
            change_detection_new: 'track_counts_off',
            changed_during_scan: false,
            control_totals: [{ column: 'amount', old: '1.50', new: '1.5' }],
          },
          ignored_columns: [{ id: 'bank.accounts.note', name: 'bank.accounts.note', reason: 'free text' }],
        },
        {
          tool_version: '1.2.3',
          mapping_hash: 'abc',
          results_sha256: 'def',
          connections: {
            old: conn({ tls: 'plaintext', plaintext_reason: 'lab', superuser: true, superuser_reason: 'rehearsal' }),
            new: conn(),
          },
        },
      ),
    );
    const r = await createTauriEngine().getResults('run-1');
    const t = r.tableResults[0];
    expect(t).toMatchObject({ oldRows: 1200, newRows: 1200, differingBuckets: 0 });
    expect(t.facts).toMatchObject({
      keyColumns: ['id'],
      bucketsTotal: 256,
      spot: { state: 'agreed', compared: 200, differing: 0 },
      changeDetectionOld: 'available',
      changeDetectionNew: 'track_counts_off',
      changedDuringScan: false,
    });
    expect(t.facts?.controlTotals).toEqual([{ column: 'amount', old: '1.50', new: '1.5' }]);
    expect(t.ignoredColumns?.[0].reason).toBe('free text');
    expect(r.runFacts).toMatchObject({ toolVersion: '1.2.3', mappingHash: 'abc', resultsSha256: 'def' });
    expect(r.runFacts?.oldConnection).toMatchObject({
      tls: 'plaintext',
      plaintextReason: 'lab',
      superuser: true,
      superuserReason: 'rehearsal',
    });
  });

  it('keeps older results loading: no facts, no connections', async () => {
    invoke.mockResolvedValueOnce(rawResults({ facts: null }));
    const r = await createTauriEngine().getResults('run-1');
    expect(r.tableResults[0].facts).toBeUndefined();
    expect(r.tableResults[0].oldRows).toBeUndefined();
    expect(r.runFacts).toBeUndefined();
  });

  it('never turns an unknown change-detection word or spot state into a clean result', async () => {
    invoke.mockResolvedValueOnce(
      rawResults({
        facts: {
          spot: { state: 'mystery', note: '' },
          change_detection_old: 'surprise',
          change_detection_new: 'available',
        },
      }),
    );
    const f = (await createTauriEngine().getResults('run-1')).tableResults[0].facts;
    expect(f?.spot?.state).toBe('disagreed');
    expect(f?.changeDetectionOld).toBe('track_counts_off');
  });
});

describe('compareStructure maps the side-by-side rows', () => {
  it('carries rows and both server versions', async () => {
    invoke.mockResolvedValueOnce({
      status: 'different',
      old_encoding: 'LATIN1',
      new_encoding: 'UTF8',
      old_collation: 'C',
      new_collation: 'C',
      entries: [],
      old_server_version: '9.6.24',
      new_server_version: '17.2',
      rows: [
        { id: 'a', depth: 2, kind: 'column', name: 'balance', status: 'changed', old: 'numeric(18,2)', new: 'numeric(19,2)', expandable: false },
        { id: 'b', depth: 2, kind: 'column', name: 'gone', status: 'missing', old: 'text', new: null, expandable: false },
      ],
    });
    const inv = await createTauriEngine().compareStructure(defaultOldProfile(), defaultOldProfile());
    expect(inv.oldServerVersion).toBe('9.6.24');
    expect(inv.rows?.[0]).toMatchObject({ old: 'numeric(18,2)', new: 'numeric(19,2)' });
    expect(inv.rows?.[1].new).toBeUndefined();
  });

  it('is fine with a backend that sends no rows', async () => {
    invoke.mockResolvedValueOnce({
      status: 'equal',
      old_encoding: 'UTF8',
      new_encoding: 'UTF8',
      old_collation: 'C',
      new_collation: 'C',
      entries: [],
    });
    const inv = await createTauriEngine().compareStructure(defaultOldProfile(), defaultOldProfile());
    expect(inv.rows).toBeUndefined();
  });

  it('shows an unknown row status as changed, never as same', () => {
    const row = translateStructureRow({
      id: 'x',
      depth: 0,
      kind: 'table',
      name: 'x',
      status: 'weird',
      old: null,
      new: null,
      expandable: false,
    });
    expect(row.status).toBe('changed');
  });
});
