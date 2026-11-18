import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { Button } from '../design/Button';
import { Input } from '../design/Input';
import type { TableCheck } from '../api/contract';
import {
  columnCounts,
  effectiveLeftOut,
  openSuggestions,
  setAllCompared,
  setColumnCompared,
  setControlTotal,
  tablePath,
} from '../screens/checkChoices';

interface ColumnChooserProps {
  table: TableCheck;
  onChange: (table: TableCheck) => void;
  onClose: () => void;
}

const levelNote: Record<TableCheck['level'], string> = {
  'full-values': 'Every value of each compared column is checked.',
  counts: 'Rows are counted. Columns you add up are summed on both sides, and the sums must match.',
  structure:
    'This table is checked for structure only: no values are read, so nothing is added up. Column choices still decide which column differences this run reports.',
};

/**
 * One table's columns: which are compared and which are added up as control
 * totals.  Key columns are always compared (rows are matched on them), so
 * their box is ticked and locked.  Adding a column up also compares it.
 */
export function ColumnChooser({ table, onChange, onClose }: ColumnChooserProps) {
  const headingRef = useRef<HTMLHeadingElement>(null);
  const [query, setQuery] = useState('');
  const searchId = useId();
  const path = tablePath(table);
  const columns = useMemo(() => table.columns ?? [], [table.columns]);
  const leftOut = new Set(effectiveLeftOut(table));
  const totals = new Set(table.controlTotalColumns ?? []);
  const suggested = new Set(table.controlTotalSuggestions ?? []);
  const { compared, total } = columnCounts(table);
  const open = openSuggestions(table);
  const canAddUp = table.level !== 'structure';

  // Move focus to the panel when it opens, so keyboard and screen-reader users
  // land on it.
  useEffect(() => {
    headingRef.current?.focus();
  }, [path]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? columns.filter((c) => `${c.name} ${c.typeName}`.toLowerCase().includes(q)) : columns;
  }, [columns, query]);

  return (
    <section
      aria-labelledby={`${searchId}-title`}
      className="space-y-3 rounded-xl border border-accent-500/40 bg-accent-500/[0.03] p-4"
      data-testid="column-chooser"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h4
            id={`${searchId}-title`}
            ref={headingRef}
            tabIndex={-1}
            className="text-[15px] font-semibold text-ink outline-none dark:text-white"
          >
            Columns of {path}
          </h4>
          <p className="text-sm text-slate-600 dark:text-slate-300" data-testid="column-summary">
            {compared === total ? `All ${total} columns are compared.` : `${compared} of ${total} columns are compared.`}{' '}
            {totals.size === 0
              ? 'Nothing is added up.'
              : `${totals.size === 1 ? '1 column is' : `${totals.size} columns are`} added up as control totals.`}
          </p>
          <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{levelNote[table.level]}</p>
        </div>
        <Button size="sm" variant="primary" onClick={onClose}>
          Done
        </Button>
      </div>

      {columns.length === 0 ? (
        <p className="text-sm text-slate-600 dark:text-slate-300">
          Dmux has no column list for this table, so every column is compared.
        </p>
      ) : (
        <>
          <div className="flex flex-wrap items-end gap-2">
            {columns.length > 10 && (
              <Input
                id={searchId}
                label="Find a column"
                placeholder="Name or type"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                className="w-56"
              />
            )}
            <Button size="sm" variant="secondary" onClick={() => onChange(setAllCompared(table, true))}>
              Compare every column
            </Button>
            <Button size="sm" variant="secondary" onClick={() => onChange(setAllCompared(table, false))}>
              Compare key columns only
            </Button>
            {canAddUp && open.length > 0 && (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => onChange(open.reduce((t, c) => setControlTotal(t, c, true), table))}
              >
                Add up the suggested {open.length === 1 ? 'column' : `${open.length} columns`}
              </Button>
            )}
          </div>
          <div className="max-h-96 overflow-auto rounded-lg border border-slate-200 dark:border-slate-700">
            <table className="w-full text-sm">
              <caption className="sr-only">Columns of {path}: compare and add up</caption>
              <thead className="sticky top-0 bg-slate-50 text-left text-xs uppercase tracking-wide text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                <tr>
                  <th scope="col" className="px-3 py-2 font-semibold">Column</th>
                  <th scope="col" className="px-3 py-2 font-semibold">Type</th>
                  <th scope="col" className="px-3 py-2 font-semibold">Compare</th>
                  <th scope="col" className="px-3 py-2 font-semibold">Add up (control total)</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((c) => {
                  const isCompared = c.key || !leftOut.has(c.name);
                  return (
                    <tr key={c.name} className="border-t border-slate-100 dark:border-slate-800">
                      <th scope="row" className="px-3 py-1.5 text-left font-medium">
                        {c.name}
                        {c.key && (
                          <span className="ml-2 rounded bg-slate-200 px-1.5 py-0.5 text-[11px] font-semibold text-slate-700 dark:bg-slate-700 dark:text-slate-200">
                            Key
                          </span>
                        )}
                      </th>
                      <td className="px-3 py-1.5 text-slate-600 dark:text-slate-300">{c.typeName}</td>
                      <td className="px-3 py-1.5">
                        <label className="flex items-center gap-2">
                          <input
                            type="checkbox"
                            className="h-4 w-4 accent-accent-600"
                            checked={isCompared}
                            disabled={c.key}
                            aria-label={`Compare ${c.name}`}
                            onChange={(e) => onChange(setColumnCompared(table, c.name, e.target.checked))}
                          />
                          <span className="text-xs text-slate-600 dark:text-slate-400">
                            {c.key ? 'Always (rows are matched on it)' : isCompared ? 'Compared' : 'Left out'}
                          </span>
                        </label>
                      </td>
                      <td className="px-3 py-1.5">
                        {c.number && canAddUp ? (
                          <label className="flex items-center gap-2">
                            <input
                              type="checkbox"
                              className="h-4 w-4 accent-accent-600"
                              checked={totals.has(c.name)}
                              aria-label={`Add up ${c.name} as a control total`}
                              onChange={(e) => onChange(setControlTotal(table, c.name, e.target.checked))}
                            />
                            {suggested.has(c.name) && (
                              <span className="text-xs text-slate-600 dark:text-slate-400">Suggested</span>
                            )}
                          </label>
                        ) : (
                          <span className="text-xs text-slate-500 dark:text-slate-400">
                            {c.number ? 'Not at this level' : 'Not a number'}
                          </span>
                        )}
                      </td>
                    </tr>
                  );
                })}
                {shown.length === 0 && (
                  <tr>
                    <td colSpan={4} className="px-3 py-3 text-slate-600 dark:text-slate-300">
                      No column matches “{query}”.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
          {leftOut.size > 0 && (
            <p className="text-xs text-slate-600 dark:text-slate-400">
              Left-out columns are listed in the results and on the report beside this table, with the
              reason “not selected for this run”, so a PASS never hides them.
            </p>
          )}
        </>
      )}
    </section>
  );
}
