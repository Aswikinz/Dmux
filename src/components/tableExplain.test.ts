import { describe, expect, it } from 'vitest';
import type { TableFacts, TableResult } from '../api/contract';
import {
  controlTotalsDiffer,
  describeChangeDetection,
  describeTls,
  sameDecimal,
  whyThisTable,
} from './tableExplain';

const facts = (over: Partial<TableFacts> = {}): TableFacts => ({
  keyColumns: ['id'],
  columns: [{ column: 'id', rule: 'compared as an exact whole number' }],
  bucketsTotal: 256,
  spot: { state: 'agreed', compared: 200, differing: 0, note: '' },
  changeDetectionOld: 'available',
  changeDetectionNew: 'available',
  changedDuringScan: false,
  controlTotals: [{ column: 'amount', old: '10.50', new: '10.5' }],
  ...over,
});

const table = (over: Partial<TableResult> = {}): TableResult => ({
  tableId: 'bank.accounts',
  schemaName: 'bank',
  tableName: 'accounts',
  verdict: 'PASS',
  level: 'full-values',
  checked: true,
  oldRows: 1200,
  newRows: 1200,
  differingBuckets: 0,
  missingBuckets: 0,
  extraBuckets: 0,
  facts: facts(),
  ...over,
});

describe('sameDecimal', () => {
  it('ignores trailing zeros and a plus sign but nothing else', () => {
    expect(sameDecimal('10.50', '10.5')).toBe(true);
    expect(sameDecimal('+3', '3')).toBe(true);
    expect(sameDecimal('0.00', '-0')).toBe(true);
    expect(sameDecimal('100', '100.00')).toBe(true);
    expect(sameDecimal('10.51', '10.5')).toBe(false);
    expect(sameDecimal('100', '10')).toBe(false);
  });

  it('trims a trailing point and keeps zeros before the point', () => {
    expect(sameDecimal('100.', '100')).toBe(true);
    expect(sameDecimal('.0', '0')).toBe(true);
    expect(sameDecimal('100', '1')).toBe(false);
    expect(sameDecimal('1000.0', '100')).toBe(false);
  });

  it('handles a very long run of zeros quickly', () => {
    const zeros = '0'.repeat(200_000);
    const start = performance.now();
    expect(sameDecimal(`1.${zeros}`, '1')).toBe(true);
    expect(sameDecimal(`1.${zeros}1`, '1')).toBe(false);
    expect(performance.now() - start).toBeLessThan(500);
  });

  it('lists only the totals that differ', () => {
    const d = controlTotalsDiffer([
      { column: 'a', old: '1.0', new: '1' },
      { column: 'b', old: '2', new: '3' },
    ]);
    expect(d.map((x) => x.column)).toEqual(['b']);
  });
});

describe('whyThisTable', () => {
  it('explains a PASS from the recorded facts only', () => {
    const w = whyThisTable(table());
    expect(w.title).toBe('Why this table passed');
    expect(w.sentence).toContain('Both databases hold 1,200 rows');
    expect(w.sentence).toContain('all 256 buckets of rows matched');
    expect(w.sentence).toContain('spot check compared 200 rows value by value, and it agreed');
    expect(w.sentence).toContain('1 control total matched');
    expect(w.sentence).toContain('did not change while it was checked');
  });

  it('explains a FAIL with the counts that differ', () => {
    const w = whyThisTable(
      table({
        verdict: 'FAIL',
        newRows: 1188,
        differingBuckets: 3,
        missingBuckets: 2,
        missingRows: 12,
        changedRows: 3,
        facts: facts({ controlTotals: [{ column: 'amount', old: '10', new: '9' }] }),
      }),
    );
    expect(w.title).toBe('Why this table failed');
    expect(w.sentence).toContain('The old database has 1,200 rows and the new one has 1,188 rows');
    expect(w.sentence).toContain('5 of 256 buckets of rows differ');
    expect(w.sentence).toContain('12 rows missing in the new database');
    expect(w.sentence).toContain('3 rows with changed values');
    expect(w.sentence).toContain('1 control total differs');
  });

  it('says the spot check disagreed when it did', () => {
    const w = whyThisTable(
      table({ verdict: 'TOOL_FAULT', facts: facts({ spot: { state: 'disagreed', note: 'x' } }) }),
    );
    expect(w.title).toBe('Why this table could not be checked');
    expect(w.sentence).toContain('spot check disagreed');
  });

  it('says change detection was unavailable for a table that needs review', () => {
    const w = whyThisTable(
      table({
        verdict: 'NEEDS_REVIEW',
        facts: facts({ changeDetectionNew: 'track_counts_off', changedDuringScan: false }),
      }),
    );
    expect(w.title).toBe('Why this table needs review');
    expect(w.sentence).toContain('change detection was not available');
  });

  it('explains a structure-only PASS without claiming rows were read', () => {
    const w = whyThisTable(table({ level: 'structure', oldRows: undefined, newRows: undefined, facts: undefined }));
    expect(w.sentence).toBe('No row was read and no structure difference was found.');
  });

  it('falls back to the first engine reason, and then to an honest "not recorded"', () => {
    const base = table({ verdict: 'WARNING', level: 'structure', oldRows: undefined, newRows: undefined, facts: undefined });
    expect(whyThisTable({ ...base, reasons: ['table exists only in the new database'] }).sentence).toBe(
      'The engine recorded: table exists only in the new database.',
    );
    expect(whyThisTable({ ...base, reasons: [] }).sentence).toBe(
      'Dmux did not record the facts behind this verdict for this table.',
    );
  });

  it('never invents a spot check or buckets that were not recorded', () => {
    const w = whyThisTable(table({ facts: facts({ spot: undefined, bucketsTotal: undefined }) }));
    expect(w.sentence).not.toContain('spot check');
    expect(w.sentence).not.toContain('bucket');
  });
});

describe('describeChangeDetection', () => {
  it('reports unchanged counters', () => {
    expect(describeChangeDetection(facts()).kind).toBe('unchanged');
  });
  it('reports moved counters', () => {
    expect(describeChangeDetection(facts({ changedDuringScan: true })).kind).toBe('moved');
  });
  it('explains an unavailable side, naming the setting', () => {
    const d = describeChangeDetection(facts({ changeDetectionOld: 'track_counts_off' }));
    expect(d.kind).toBe('unavailable');
    expect('text' in d && d.text).toContain('track_counts');
    expect('text' in d && d.text).toContain('old database');
    expect('text' in d && d.text).toContain('needs review');
  });
  it('explains a foreign table', () => {
    const d = describeChangeDetection(facts({ changeDetectionNew: 'foreign_table' }));
    expect('text' in d && d.text).toContain('foreign table');
  });
  it('is unrecorded without facts', () => {
    expect(describeChangeDetection(undefined).kind).toBe('unrecorded');
  });
  it('does not claim "unchanged" when no before-and-after result exists', () => {
    expect(describeChangeDetection(facts({ changedDuringScan: undefined })).kind).toBe('not_checked');
  });
});

describe('describeTls', () => {
  const c = (tls: string, extra: Record<string, string> = {}) => ({
    tls,
    superuser: false,
    canWrite: false,
    serverVersion: '17',
    serverEncoding: 'UTF8',
    ...extra,
  });
  it('speaks plainly about every state', () => {
    expect(describeTls(c('verified'))).toContain('verified');
    expect(describeTls(c('pinned'))).toContain('fingerprint');
    expect(describeTls(c('plaintext', { plaintextReason: 'lab network' }))).toContain('lab network');
    expect(describeTls(c(''))).toContain('capture file');
    expect(describeTls(c('weird'))).toContain('Unrecognised');
  });
});
