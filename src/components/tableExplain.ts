import type {
  ChangeDetectionState,
  ConnectionFact,
  ControlTotalFact,
  TableFacts,
  TableResult,
  Verdict,
} from '../api/contract';

/** What each check level means, in plain words. */
export const LEVEL_MEANING: Record<TableResult['level'], string> = {
  structure:
    "Dmux compared the table's definition: its columns, their types, keys, indexes and constraints. It did not read any rows.",
  counts:
    'Dmux counted the rows on both sides and added up the number columns you chose. It did not compare individual values.',
  'full-values':
    'Dmux compared every value in every compared column of every row. It does not depend on the order of the rows.',
};

const num = (n: number): string => n.toLocaleString('en-US');
const plural = (n: number, one: string, many = `${one}s`): string => `${num(n)} ${n === 1 ? one : many}`;

/** Compare two decimal strings exactly, ignoring trailing zeros (1.10 equals 1.1). */
export function sameDecimal(a: string, b: string): boolean {
  const norm = (s: string): string => {
    let t = s.trim().replace(/^\+/, '');
    if (t.includes('.')) {
      // Drop trailing zeros, then a bare trailing point.  A loop, not /0+$/,
      // which backtracks badly on long runs of zeros.
      let end = t.length;
      while (end > 0 && t[end - 1] === '0') end -= 1;
      if (end > 0 && t[end - 1] === '.') end -= 1;
      t = t.slice(0, end);
    }
    t = t.replace(/^(-?)0+(?=\d)/, '$1');
    return t === '-0' || t === '' ? '0' : t;
  };
  return norm(a) === norm(b);
}

export function controlTotalsDiffer(totals: ControlTotalFact[]): ControlTotalFact[] {
  return totals.filter((t) => !sameDecimal(t.old, t.new));
}

/** Why change detection could not vouch for one side, in a clause. */
function whyUnavailable(side: 'old' | 'new', state: ChangeDetectionState): string | null {
  if (state === 'track_counts_off') {
    return `the database setting "track_counts" is off on the ${side} database`;
  }
  if (state === 'foreign_table') {
    return `the ${side} table is a foreign table, which has no change counters`;
  }
  return null;
}

export type ChangeDetectionSummary =
  | { kind: 'unrecorded' }
  | { kind: 'unavailable'; text: string }
  | { kind: 'unchanged'; text: string }
  | { kind: 'moved'; text: string }
  | { kind: 'not_checked'; text: string };

export function describeChangeDetection(f: TableFacts | undefined): ChangeDetectionSummary {
  if (!f || (f.changeDetectionOld === undefined && f.changeDetectionNew === undefined)) {
    return { kind: 'unrecorded' };
  }
  const reasons = [
    f.changeDetectionOld ? whyUnavailable('old', f.changeDetectionOld) : null,
    f.changeDetectionNew ? whyUnavailable('new', f.changeDetectionNew) : null,
  ].filter((r): r is string => r !== null);
  if (reasons.length > 0) {
    return {
      kind: 'unavailable',
      text: `Change detection was not available: ${reasons.join(', and ')}. Dmux cannot rule out that the table changed while it was read, so the table needs review instead of passing.`,
    };
  }
  if (f.changedDuringScan === true) {
    return {
      kind: 'moved',
      text: 'The insert, update or delete counters moved while the table was read, so the table changed during the check.',
    };
  }
  if (f.changedDuringScan === false) {
    return {
      kind: 'unchanged',
      text: 'The insert, update and delete counters of both tables were the same before and after the table was read, so nothing changed while it was checked.',
    };
  }
  return {
    kind: 'not_checked',
    text: 'Both databases can detect changes, but no before-and-after result was recorded for this table.',
  };
}

const WHY_VERB: Record<Verdict, string> = {
  PASS: 'passed',
  FAIL: 'failed',
  WIDELY_DIFFERENT: 'is widely different',
  NEEDS_REVIEW: 'needs review',
  CHANGED_DURING_CHECK: 'changed during the check',
  TOOL_FAULT: 'could not be checked',
  WARNING: 'has a warning',
};

export interface WhyTable {
  /** "Why this table passed", "Why this table failed", ... */
  title: string;
  /** One sentence built only from recorded facts. */
  sentence: string;
}

/**
 * The one-sentence explanation of a table's verdict.  Every clause comes from a
 * recorded count, flag or engine reason; nothing is inferred from the verdict.
 */
export function whyThisTable(t: TableResult): WhyTable {
  const title = `Why this table ${WHY_VERB[t.verdict]}`;
  const f = t.facts;
  const clauses: string[] = [];

  if (t.oldRows !== undefined && t.newRows !== undefined) {
    clauses.push(
      t.oldRows === t.newRows
        ? `both databases hold ${plural(t.oldRows, 'row')}`
        : `the old database has ${plural(t.oldRows, 'row')} and the new one has ${plural(t.newRows, 'row')}`,
    );
  }
  if (t.level === 'full-values') {
    if (f?.bucketsTotal !== undefined) {
      const differ = (t.differingBuckets ?? 0) + (t.missingBuckets ?? 0) + (t.extraBuckets ?? 0);
      clauses.push(
        differ === 0
          ? `all ${plural(f.bucketsTotal, 'bucket')} of rows matched`
          : `${num(differ)} of ${plural(f.bucketsTotal, 'bucket')} of rows differ`,
      );
    }
    const rowDiffs = [
      t.missingRows ? `${plural(t.missingRows, 'row')} missing in the new database` : '',
      t.extraRows ? `${plural(t.extraRows, 'row')} only in the new database` : '',
      t.changedRows ? `${plural(t.changedRows, 'row')} with changed values` : '',
    ].filter(Boolean);
    if (rowDiffs.length > 0) clauses.push(rowDiffs.join(', '));
    if (f?.spot?.state === 'agreed') {
      clauses.push(
        `the spot check compared ${plural(f.spot.compared ?? 0, 'row')} value by value, and it agreed with the bucket comparison`,
      );
    } else if (f?.spot?.state === 'disagreed') {
      clauses.push('the spot check disagreed with the fingerprint, which points at a fault in Dmux');
    } else if (f?.spot?.state === 'not_run') {
      clauses.push('the spot check did not run');
    }
  }
  if (t.level !== 'structure' && f && f.controlTotals.length > 0) {
    const differ = controlTotalsDiffer(f.controlTotals).length;
    clauses.push(
      differ === 0
        ? `${plural(f.controlTotals.length, 'control total')} matched`
        : `${plural(differ, 'control total')} ${differ === 1 ? 'differs' : 'differ'}`,
    );
  }
  const cd = describeChangeDetection(f);
  if (cd.kind === 'unchanged') clauses.push('the table did not change while it was checked');
  if (cd.kind === 'moved') clauses.push('the table changed while it was checked');
  if (cd.kind === 'unavailable') clauses.push('change detection was not available');

  if (t.level === 'structure' && t.verdict === 'PASS') {
    clauses.unshift('no row was read and no structure difference was found');
  }
  if (clauses.length === 0) {
    const first = t.reasons?.[0];
    return {
      title,
      sentence: first
        ? `The engine recorded: ${first}.`
        : 'Dmux did not record the facts behind this verdict for this table.',
    };
  }
  const [head, ...rest] = clauses;
  const body = [head, ...rest].join('; ');
  return { title, sentence: `${body.charAt(0).toUpperCase()}${body.slice(1)}.` };
}

/** TLS state of one connection, in plain words. */
export function describeTls(c: ConnectionFact): string {
  switch (c.tls) {
    case 'verified':
      return 'Encrypted, and the server certificate was verified';
    case 'pinned':
      return 'Encrypted, and the server certificate was trusted once by its fingerprint';
    case 'plaintext':
      return c.plaintextReason
        ? `Not encrypted. Reason you recorded: ${c.plaintextReason}`
        : 'Not encrypted';
    case '':
      return 'Not a live connection (a saved capture file)';
    default:
      return `Unrecognised connection security: ${c.tls}`;
  }
}
