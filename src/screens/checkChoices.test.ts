import { describe, expect, it } from 'vitest';
import type { CheckPlan, TableCheck } from '../api/contract';
import {
  applyChoices,
  columnCounts,
  describeApply,
  effectiveLeftOut,
  planToChoices,
  setColumnCompared,
  setControlTotal,
} from './checkChoices';

const table = (name: string, extra: Partial<TableCheck> = {}): TableCheck => ({
  tableId: `core.${name}`,
  schemaName: 'core',
  tableName: name,
  level: 'full-values',
  columns: [
    { name: 'id', typeName: 'integer', key: true, number: true },
    { name: 'amount', typeName: 'numeric', key: false, number: true },
    { name: 'note', typeName: 'text', key: false, number: false },
  ],
  controlTotalSuggestions: ['amount'],
  ...extra,
});

const plan = (tables: TableCheck[]): CheckPlan => ({
  tables,
  presets: [],
  estimateSecondsMin: 1,
  estimateSecondsMax: 2,
});

describe('column choices', () => {
  it('a key column is always compared, whatever the list says', () => {
    const t = table('loans', { leftOutColumns: ['id', 'note'] });
    expect(effectiveLeftOut(t)).toEqual(['note']);
    expect(columnCounts(t)).toEqual({ compared: 2, total: 3 });
    expect(setColumnCompared(t, 'id', false)).toBe(t);
  });

  it('a control total is always a compared column', () => {
    const t = setControlTotal(table('loans', { leftOutColumns: ['amount'] }), 'amount', true);
    expect(t.leftOutColumns).toBeUndefined();
    expect(t.controlTotalColumns).toEqual(['amount']);
    const out = setColumnCompared(t, 'amount', false);
    expect(out.controlTotalColumns).toBeUndefined();
  });
});

describe('saved check choices', () => {
  it('saves only the chosen tables, with their levels and columns', () => {
    const p = plan([
      table('loans', { level: 'counts', leftOutColumns: ['note', 'id'], controlTotalColumns: ['amount'] }),
      table('archive', { included: false }),
    ]);
    p.runWindow = '22:00-05:00';
    expect(planToChoices(p)).toEqual({
      runWindow: '22:00-05:00',
      tables: [{ table: 'core.loans', level: 'counts', leftOutColumns: ['note'], controlTotals: ['amount'] }],
    });
  });

  it('opening the saved choices gives back the same run', () => {
    const p = plan([
      table('loans', { level: 'counts', leftOutColumns: ['note'], controlTotalColumns: ['amount'] }),
      table('archive', { included: false }),
      table('fees', { level: 'structure' }),
    ]);
    const fresh = plan(p.tables.map((t) => ({ ...t, included: false, leftOutColumns: undefined, controlTotalColumns: undefined, level: 'full-values' })));
    const { plan: back, report } = applyChoices(fresh, planToChoices(p));
    expect(planToChoices(back)).toEqual(planToChoices(p));
    expect(report).toEqual({ applied: 2, missingTables: [], missingColumns: [], keysKept: [], notNumbers: [] });
  });

  it('tables the file does not name are not chosen', () => {
    const p = plan([table('loans'), table('fees', { leftOutColumns: ['note'] })]);
    const { plan: back } = applyChoices(p, {
      tables: [{ table: 'core.loans', level: 'counts', leftOutColumns: [], controlTotals: [] }],
    });
    expect(back.tables.map((t) => t.included)).toEqual([true, false]);
    expect(back.tables[1].leftOutColumns).toBeUndefined();
  });

  it('reports every name it could not use, never guessing', () => {
    const p = plan([table('loans')]);
    const { plan: back, report } = applyChoices(p, {
      tables: [
        { table: 'core.loans', level: 'full-values', leftOutColumns: ['id', 'gone'], controlTotals: ['note', 'missing'] },
        { table: 'core.vanished', level: 'counts', leftOutColumns: [], controlTotals: [] },
      ],
    });
    expect(report.missingTables).toEqual(['core.vanished']);
    expect(report.missingColumns).toEqual(['core.loans.gone', 'core.loans.missing']);
    expect(report.keysKept).toEqual(['core.loans.id']);
    expect(report.notNumbers).toEqual(['core.loans.note']);
    expect(back.tables[0].leftOutColumns).toBeUndefined();
    expect(back.tables[0].controlTotalColumns).toBeUndefined();
    const words = describeApply(report).join(' ');
    expect(words).toContain('1 table in the file is not in this mapping, so it is not checked: core.vanished.');
    expect(words).toContain('2 columns in the file do not exist here and were skipped');
    expect(words).toContain('Key columns are always compared');
    expect(words).toContain('Only number columns can be added up');
  });
});
