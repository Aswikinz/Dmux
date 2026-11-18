import type { CheckChoices, CheckPlan, PlanColumn, TableCheck } from '../api/contract';

/** A table's `schema.table`. */
export const tablePath = (t: TableCheck): string => `${t.schemaName}.${t.tableName}`;

/** The key columns of a table: always compared, never left out. */
export const keyColumns = (t: TableCheck): Set<string> =>
  new Set((t.columns ?? []).filter((c) => c.key).map((c) => c.name));

/** The columns a left-out list may really leave out: never a key column. */
export function effectiveLeftOut(t: TableCheck): string[] {
  const keys = keyColumns(t);
  return (t.leftOutColumns ?? []).filter((c) => !keys.has(c));
}

/** How many of a table's columns are compared, out of how many. */
export function columnCounts(t: TableCheck): { compared: number; total: number } {
  const total = t.columns?.length ?? 0;
  return { compared: total - effectiveLeftOut(t).length, total };
}

/** Leave a column in or out.  A key column always stays in. */
export function setColumnCompared(t: TableCheck, column: string, compared: boolean): TableCheck {
  if (keyColumns(t).has(column)) return t;
  const out = new Set(t.leftOutColumns ?? []);
  if (compared) out.delete(column);
  else out.add(column);
  // A column that is not compared cannot be added up either.
  const totals = compared
    ? t.controlTotalColumns
    : (t.controlTotalColumns ?? []).filter((c) => c !== column);
  return {
    ...t,
    leftOutColumns: out.size > 0 ? [...out].sort() : undefined,
    controlTotalColumns: totals && totals.length > 0 ? totals : undefined,
  };
}

/** Compare every column, or only the key columns. */
export function setAllCompared(t: TableCheck, compared: boolean): TableCheck {
  if (compared) return { ...t, leftOutColumns: undefined };
  const keys = keyColumns(t);
  const out = (t.columns ?? []).filter((c) => !keys.has(c.name)).map((c) => c.name);
  return {
    ...t,
    leftOutColumns: out.length > 0 ? out : undefined,
    controlTotalColumns: (t.controlTotalColumns ?? []).filter((c) => keys.has(c)).length
      ? (t.controlTotalColumns ?? []).filter((c) => keys.has(c))
      : undefined,
  };
}

/** Add up a column (or stop adding it up) as a control total for this table
 *  only.  Adding one up also puts it back into the comparison. */
export function setControlTotal(t: TableCheck, column: string, on: boolean): TableCheck {
  const totals = new Set(t.controlTotalColumns ?? []);
  if (on) totals.add(column);
  else totals.delete(column);
  const base = on ? setColumnCompared(t, column, true) : t;
  return { ...base, controlTotalColumns: totals.size > 0 ? [...totals].sort() : undefined };
}

/** The number columns Dmux suggests adding up that are still compared. */
export function openSuggestions(t: TableCheck): string[] {
  const out = new Set(effectiveLeftOut(t));
  const on = new Set(t.controlTotalColumns ?? []);
  return (t.controlTotalSuggestions ?? []).filter((c) => !out.has(c) && !on.has(c));
}

/** Add up every suggested column in the given tables. */
export function addSuggestedTotals(plan: CheckPlan, ids: Set<string>): CheckPlan {
  return {
    ...plan,
    tables: plan.tables.map((t) =>
      ids.has(t.tableId)
        ? openSuggestions(t).reduce((acc, c) => setControlTotal(acc, c, true), t)
        : t,
    ),
  };
}

/** Stop adding up anything in the given tables. */
export function clearTotals(plan: CheckPlan, ids: Set<string>): CheckPlan {
  return {
    ...plan,
    tables: plan.tables.map((t) => (ids.has(t.tableId) ? { ...t, controlTotalColumns: undefined } : t)),
  };
}

/** The chosen tables' choices, ready to save.  Tables not chosen are left out
 *  of the file: opening it again leaves them out of the run. */
export function planToChoices(plan: CheckPlan): CheckChoices {
  return {
    runWindow: plan.runWindow,
    tables: plan.tables
      .filter((t) => t.included !== false)
      .map((t) => ({
        table: tablePath(t),
        level: t.level,
        leftOutColumns: effectiveLeftOut(t),
        controlTotals: [...(t.controlTotalColumns ?? [])].sort(),
      })),
  };
}

/** What happened when saved choices were applied, so nothing is dropped
 *  silently. */
export interface ApplyReport {
  /** Tables in the file that this run chose. */
  applied: number;
  /** Tables in the file that this database does not have (or are not mapped). */
  missingTables: string[];
  /** `schema.table.column` names in the file this database does not have. */
  missingColumns: string[];
  /** Key columns the file left out: they are compared anyway. */
  keysKept: string[];
  /** Control totals on columns that are not number columns: not added up. */
  notNumbers: string[];
}

const byName = (t: TableCheck): Map<string, PlanColumn> =>
  new Map((t.columns ?? []).map((c) => [c.name, c]));

/** Apply saved choices to this plan.  Every table the file names and the plan
 *  has is chosen with the file's level and columns; every other table is not
 *  chosen.  Names the plan does not have are reported, never guessed. */
export function applyChoices(plan: CheckPlan, choices: CheckChoices): { plan: CheckPlan; report: ApplyReport } {
  const wanted = new Map(choices.tables.map((c) => [c.table, c]));
  const known = new Set(plan.tables.map(tablePath));
  const report: ApplyReport = {
    applied: 0,
    missingTables: choices.tables.map((c) => c.table).filter((t) => !known.has(t)),
    missingColumns: [],
    keysKept: [],
    notNumbers: [],
  };
  const tables = plan.tables.map((t): TableCheck => {
    const path = tablePath(t);
    const choice = wanted.get(path);
    if (!choice) {
      return { ...t, included: false, leftOutColumns: undefined, controlTotalColumns: undefined };
    }
    report.applied += 1;
    const cols = byName(t);
    const has = (c: string) => cols.size === 0 || cols.has(c);
    const leftOut: string[] = [];
    for (const c of choice.leftOutColumns) {
      if (!has(c)) report.missingColumns.push(`${path}.${c}`);
      else if (cols.get(c)?.key) report.keysKept.push(`${path}.${c}`);
      else leftOut.push(c);
    }
    const totals: string[] = [];
    for (const c of choice.controlTotals) {
      if (!has(c)) report.missingColumns.push(`${path}.${c}`);
      else if (cols.size > 0 && !cols.get(c)?.number) report.notNumbers.push(`${path}.${c}`);
      else totals.push(c);
    }
    return {
      ...t,
      included: true,
      level: choice.level,
      leftOutColumns: leftOut.length > 0 ? leftOut.sort() : undefined,
      controlTotalColumns: totals.length > 0 ? totals.sort() : undefined,
    };
  });
  return { plan: { ...plan, tables, runWindow: choices.runWindow ?? plan.runWindow }, report };
}

const listed = (names: string[]): string => {
  const shown = names.slice(0, 5).join(', ');
  return names.length > 5 ? `${shown} and ${names.length - 5} more` : shown;
};

/** The apply report in plain words, one sentence per kind of problem. */
export function describeApply(report: ApplyReport): string[] {
  const lines = [
    report.applied === 1
      ? 'Applied the saved choices to 1 table.'
      : `Applied the saved choices to ${report.applied} tables.`,
  ];
  if (report.missingTables.length > 0) {
    lines.push(
      `${report.missingTables.length === 1 ? '1 table' : `${report.missingTables.length} tables`} in the file ${report.missingTables.length === 1 ? 'is' : 'are'} not in this mapping, so ${report.missingTables.length === 1 ? 'it is' : 'they are'} not checked: ${listed(report.missingTables)}.`,
    );
  }
  if (report.missingColumns.length > 0) {
    lines.push(
      `${report.missingColumns.length === 1 ? '1 column' : `${report.missingColumns.length} columns`} in the file ${report.missingColumns.length === 1 ? 'does' : 'do'} not exist here and ${report.missingColumns.length === 1 ? 'was' : 'were'} skipped: ${listed(report.missingColumns)}.`,
    );
  }
  if (report.keysKept.length > 0) {
    lines.push(
      `Key columns are always compared, because rows are matched on them, so these stay in: ${listed(report.keysKept)}.`,
    );
  }
  if (report.notNumbers.length > 0) {
    lines.push(
      `Only number columns can be added up, so these are not control totals: ${listed(report.notNumbers)}.`,
    );
  }
  return lines;
}
