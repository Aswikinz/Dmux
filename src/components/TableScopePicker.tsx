import { useEffect, useMemo, useRef, useState } from 'react';
import { ChevronDown, ChevronRight, Search } from 'lucide-react';
import { Button } from '../design/Button';
import { cn } from '../design/cn';
import type { TableCheck } from '../api/contract';
import { formatSize } from '../screens/checkSelection';

/** A table is in the run unless it was explicitly left out. */
export const isChosen = (t: TableCheck): boolean => t.included !== false;

/** Set `included` on every table whose id is in `ids` (others unchanged). */
export function setChosen(tables: TableCheck[], ids: Set<string>, on: boolean): TableCheck[] {
  return tables.map((t) => (ids.has(t.tableId) ? { ...t, included: on } : t));
}

interface TableScopePickerProps {
  tables: TableCheck[];
  onChange: (tables: TableCheck[]) => void;
}

/** A checkbox that can show "some chosen" (indeterminate). */
function TriCheckbox({
  checked,
  partial,
  label,
  onChange,
}: {
  checked: boolean;
  partial: boolean;
  label: string;
  onChange: (on: boolean) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = partial;
  }, [partial]);
  return (
    <input
      ref={ref}
      type="checkbox"
      className="h-4 w-4 shrink-0 accent-accent-600"
      aria-label={label}
      aria-checked={partial ? 'mixed' : checked}
      checked={checked}
      onChange={(e) => onChange(e.target.checked)}
    />
  );
}

/**
 * Choose which schemas and tables a run reads.  Nothing is chosen until the
 * user picks: a whole schema with its box, single tables inside it, or every
 * table with "Select all".  Tables left out are never read; the run records
 * each one as "not selected for this run", so the coverage line and every
 * report still name them.
 */
export function TableScopePicker({ tables, onChange }: TableScopePickerProps) {
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState<Set<string>>(new Set());

  const bySchema = useMemo(() => {
    const groups = new Map<string, TableCheck[]>();
    for (const t of tables) {
      const list = groups.get(t.schemaName) ?? [];
      list.push(t);
      groups.set(t.schemaName, list);
    }
    for (const list of groups.values()) {
      list.sort((a, b) => a.tableName.localeCompare(b.tableName));
    }
    return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [tables]);

  const q = query.trim().toLowerCase();
  const matches = (t: TableCheck) =>
    !q || `${t.schemaName}.${t.tableName}`.toLowerCase().includes(q);
  const chosenCount = tables.filter(isChosen).length;
  const matching = q ? tables.filter(matches) : [];

  const choose = (list: TableCheck[], on: boolean) =>
    onChange(setChosen(tables, new Set(list.map((t) => t.tableId)), on));

  const toggleOpen = (schema: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(schema)) next.delete(schema);
      else next.add(schema);
      return next;
    });

  return (
    <section aria-labelledby="scope-title" className="flex flex-col gap-4" data-testid="table-scope">
      <div>
        <h3 id="scope-title" className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">
          Tables to check
        </h3>
        <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">
          Choose the schemas and tables this run reads. During busy hours, pick only a few. Tables
          you leave out are not read; the results and every report list them as &ldquo;not
          selected for this run&rdquo;.
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <label className="relative min-w-[16rem] flex-1">
          <span className="sr-only">Find a schema or table</span>
          <Search
            className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-400"
            aria-hidden="true"
          />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Find a schema or table"
            className="h-9 w-full rounded-[10px] border border-black/[0.1] bg-white pl-9 pr-3 text-sm text-ink focus:border-accent-500 focus:outline-none focus:ring-4 focus:ring-accent-500/20 dark:border-white/10 dark:bg-white/[0.06] dark:text-slate-100"
          />
        </label>
        {q && (
          <Button
            size="sm"
            variant="secondary"
            disabled={matching.length === 0}
            onClick={() => choose(matching, true)}
          >
            Select {matching.length} matching
          </Button>
        )}
        <Button size="sm" variant="secondary" onClick={() => choose(tables, true)}>
          Select all
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={chosenCount === 0}
          onClick={() => choose(tables, false)}
        >
          Clear all
        </Button>
      </div>

      <p role="status" className="text-sm font-medium text-slate-700 dark:text-slate-200" data-testid="scope-count">
        {chosenCount === 0
          ? `No tables chosen yet (${tables.length} available).`
          : chosenCount === tables.length
            ? `All ${tables.length} tables chosen.`
            : `${chosenCount} of ${tables.length} tables chosen. The other ${tables.length - chosenCount} will not be read.`}
      </p>

      <ul className="divide-y divide-black/[0.06] rounded-xl border border-black/[0.08] dark:divide-white/[0.06] dark:border-white/10">
        {bySchema.map(([schema, list]) => {
          const shown = list.filter(matches);
          if (q && shown.length === 0) return null;
          const n = list.filter(isChosen).length;
          const expanded = open.has(schema) || !!q;
          const panelId = `scope-${schema}`;
          return (
            <li key={schema}>
              <div className="flex items-center gap-3 px-3 py-2.5">
                <button
                  type="button"
                  onClick={() => toggleOpen(schema)}
                  aria-expanded={expanded}
                  aria-controls={panelId}
                  aria-label={`${expanded ? 'Hide' : 'Show'} the tables in ${schema}`}
                  className="rounded p-0.5 text-slate-500 hover:bg-black/[0.05] dark:text-slate-400 dark:hover:bg-white/[0.08]"
                >
                  {expanded ? (
                    <ChevronDown className="h-4 w-4" aria-hidden="true" />
                  ) : (
                    <ChevronRight className="h-4 w-4" aria-hidden="true" />
                  )}
                </button>
                <TriCheckbox
                  checked={n === list.length}
                  partial={n > 0 && n < list.length}
                  label={`Every table in ${schema}`}
                  onChange={(on) => choose(list, on)}
                />
                <button
                  type="button"
                  onClick={() => toggleOpen(schema)}
                  className="flex-1 text-left text-sm font-semibold text-ink dark:text-slate-100"
                >
                  {schema}
                </button>
                <span className="text-xs text-slate-500 dark:text-slate-400">
                  {n} of {list.length} chosen
                </span>
              </div>
              {expanded && (
                <div
                  id={panelId}
                  className="max-h-72 overflow-auto border-t border-black/[0.04] bg-black/[0.015] px-3 py-2 dark:border-white/[0.05] dark:bg-white/[0.02]"
                >
                  <div className="grid gap-x-6 gap-y-1 pl-7 sm:grid-cols-2 xl:grid-cols-3">
                    {shown.map((t) => (
                      <label
                        key={t.tableId}
                        className={cn(
                          'flex min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-sm hover:bg-black/[0.04] dark:hover:bg-white/[0.05]',
                          isChosen(t) ? 'text-ink dark:text-slate-100' : 'text-slate-600 dark:text-slate-400',
                        )}
                      >
                        <input
                          type="checkbox"
                          className="h-4 w-4 shrink-0 accent-accent-600"
                          checked={isChosen(t)}
                          onChange={(e) => choose([t], e.target.checked)}
                          aria-label={`Check ${t.schemaName}.${t.tableName}`}
                        />
                        <span className="truncate">{t.tableName}</span>
                        {t.sizeBytes !== undefined && (
                          <span className="ml-auto shrink-0 text-xs text-slate-500 dark:text-slate-400">
                            {formatSize(t.sizeBytes)}
                          </span>
                        )}
                      </label>
                    ))}
                  </div>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
