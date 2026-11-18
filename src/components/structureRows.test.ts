import { describe, expect, it } from 'vitest';
import type { SchemaDiffNode, StructureRow } from '../api/contract';
import { largeInventory } from '../api/mockEngine';
import { rowsFromNodes, visibleRows } from './structureRows';

const row = (over: Partial<StructureRow> & Pick<StructureRow, 'id' | 'depth' | 'name'>): StructureRow => ({
  kind: 'column',
  status: 'same',
  old: 'text',
  new: 'text',
  expandable: false,
  ...over,
});

const sample: StructureRow[] = [
  row({ id: 's', depth: 0, name: 'bank', kind: 'schema', expandable: true, old: 'present', new: 'present' }),
  row({ id: 's/t1', depth: 1, name: 'accounts', kind: 'table', expandable: true }),
  row({ id: 's/t1/id', depth: 2, name: 'id', old: 'bigint', new: 'bigint' }),
  row({ id: 's/t1/bal', depth: 2, name: 'balance', status: 'changed', old: 'numeric(18,2)', new: 'numeric(19,2)' }),
  row({ id: 's/t1/gone', depth: 2, name: 'legacy_flag', status: 'missing', new: undefined }),
  row({ id: 's/t1/memo', depth: 2, name: 'memo', status: 'added', old: undefined, new: 'text' }),
  row({ id: 's/t2', depth: 1, name: 'branches', kind: 'table', expandable: true }),
  row({ id: 's/t2/code', depth: 2, name: 'code' }),
];

const ids = (r: { rows: { id: string }[] }) => r.rows.map((x) => x.id);
const none = new Set<string>();

describe('rowsFromNodes', () => {
  it('flattens a tree into aligned rows with gaps on the missing side', () => {
    const nodes: SchemaDiffNode[] = [
      {
        id: 'a',
        kind: 'schema',
        name: 'bank',
        status: 'same',
        children: [
          { id: 'b', kind: 'table', name: 'gone', status: 'missing' },
          { id: 'c', kind: 'table', name: 'fresh', status: 'added' },
          { id: 'd', kind: 'column', name: 'x', status: 'changed', old: 'integer', new: 'text' },
        ],
      },
    ];
    const rows = rowsFromNodes(nodes);
    expect(rows.map((r) => r.depth)).toEqual([0, 1, 1, 1]);
    expect(rows[0].expandable).toBe(true);
    expect(rows[1]).toMatchObject({ status: 'missing', new: undefined });
    expect(rows[1].old).toBeDefined();
    expect(rows[2]).toMatchObject({ status: 'added', old: undefined });
    expect(rows[3]).toMatchObject({ old: 'integer', new: 'text' });
  });
});

describe('visibleRows', () => {
  it('opens schemas and groups with differences, and closes the rest', () => {
    const r = visibleRows(sample, { toggled: none, onlyProblems: false, query: '' });
    // accounts has differences below it, so it is open; branches has none, so it is closed.
    expect(ids(r)).toEqual(['s', 's/t1', 's/t1/id', 's/t1/bal', 's/t1/gone', 's/t1/memo', 's/t2']);
    expect(r.differences).toBe(3);
    expect(r.total).toBe(sample.length);
    expect(r.rows.find((x) => x.id === 's/t2')?.expanded).toBe(false);
  });

  it('keeps a missing item and an added item on their own side', () => {
    const r = visibleRows(sample, { toggled: none, onlyProblems: false, query: '' });
    const gone = r.rows.find((x) => x.id === 's/t1/gone');
    const memo = r.rows.find((x) => x.id === 's/t1/memo');
    expect(gone?.old).toBeDefined();
    expect(gone?.new).toBeUndefined();
    expect(memo?.old).toBeUndefined();
    expect(memo?.new).toBeDefined();
  });

  it('toggling flips a group from its default', () => {
    const open = visibleRows(sample, { toggled: new Set(['s/t2']), onlyProblems: false, query: '' });
    expect(ids(open)).toContain('s/t2/code');
    const closed = visibleRows(sample, { toggled: new Set(['s/t1']), onlyProblems: false, query: '' });
    expect(ids(closed)).not.toContain('s/t1/id');
  });

  it('only problems keeps differing rows and the rows above them', () => {
    const r = visibleRows(sample, { toggled: none, onlyProblems: true, query: '' });
    expect(ids(r)).toEqual(['s', 's/t1', 's/t1/bal', 's/t1/gone', 's/t1/memo']);
  });

  it('search keeps matches, the path to them and the rows under a match', () => {
    const col = visibleRows(sample, { toggled: none, onlyProblems: false, query: 'CODE' });
    expect(ids(col)).toEqual(['s', 's/t2', 's/t2/code']);
    const table = visibleRows(sample, { toggled: new Set(['s/t1']), onlyProblems: false, query: 'accounts' });
    expect(ids(table)).toContain('s/t1');
    expect(ids(table)).not.toContain('s/t2');
  });

  it('search also matches the definition on either side', () => {
    const r = visibleRows(sample, { toggled: none, onlyProblems: false, query: 'numeric(19' });
    expect(ids(r)).toEqual(['s', 's/t1', 's/t1/bal']);
  });

  it('search and only problems combine', () => {
    const r = visibleRows(sample, { toggled: none, onlyProblems: true, query: 'code' });
    expect(r.rows).toEqual([]);
  });

  it('handles an empty list', () => {
    expect(visibleRows([], { toggled: none, onlyProblems: false, query: 'x' })).toEqual({
      rows: [],
      total: 0,
      differences: 0,
      hiddenDifferences: 0,
    });
  });

  it('hides kinds of object, keeps the path to what is shown, and counts what it hid', () => {
    const rows: StructureRow[] = [
      row({ id: 's', depth: 0, name: 'bank', kind: 'schema', expandable: true, old: 'present', new: 'present' }),
      row({ id: 's/t1', depth: 1, name: 'accounts', kind: 'table', expandable: true }),
      row({ id: 's/t1/bal', depth: 2, name: 'balance', status: 'changed', old: 'a', new: 'b' }),
      row({ id: 's/t1/ix', depth: 2, name: 'accounts_pkey', kind: 'index', status: 'changed', old: 'a', new: 'b' }),
      row({ id: 's/fn', depth: 1, name: 'hash_ltree', kind: 'function', status: 'added', old: undefined }),
      row({ id: 's/ext', depth: 1, name: 'ltree', kind: 'extension', status: 'added', old: undefined }),
    ];
    const opts = { toggled: none, onlyProblems: false, query: '' };
    const all = visibleRows(rows, opts);
    expect(all.differences).toBe(4);
    expect(all.hiddenDifferences).toBe(0);

    const tablesOnly = visibleRows(rows, { ...opts, hiddenGroups: new Set(['indexes', 'other'] as const) });
    expect(ids(tablesOnly)).toEqual(['s', 's/t1', 's/t1/bal']);
    expect(tablesOnly.differences).toBe(1);
    expect(tablesOnly.hiddenDifferences).toBe(3);

    // Hiding tables keeps an index visible under its table, as a path.
    const indexesOnly = visibleRows(rows, { ...opts, hiddenGroups: new Set(['tables', 'other'] as const) });
    expect(ids(indexesOnly)).toEqual(['s', 's/t1', 's/t1/ix']);
  });

  it('stays fast with thousands of tables', () => {
    const rows = largeInventory(5000).rows ?? [];
    expect(rows.length).toBeGreaterThan(25000);
    const start = performance.now();
    const plain = visibleRows(rows, { toggled: none, onlyProblems: false, query: '' });
    const search = visibleRows(rows, { toggled: none, onlyProblems: false, query: 'table_04999' });
    const problems = visibleRows(rows, { toggled: none, onlyProblems: true, query: '' });
    const elapsed = performance.now() - start;
    expect(plain.rows.length).toBeGreaterThan(5000);
    expect(search.rows.some((r) => r.name === 'table_04999')).toBe(true);
    expect(problems.rows.length).toBeLessThan(plain.rows.length);
    expect(elapsed).toBeLessThan(1500);
  });
});
