import { useMemo, useState } from 'react';
import { ArrowRight, ChevronDown, ChevronRight, Search } from 'lucide-react';
import { cn } from '../design/cn';
import type { MappingMatch } from '../api/contract';

/** How a pair was made, in plain words. */
export function howMatched(m: MappingMatch): string {
  if (!m.approved) return 'Needs your decision';
  switch (m.confidence) {
    case 'exact':
      return 'Same name';
    case 'case':
      return 'Same name, different capitals';
    case 'rule':
      return 'Renamed by a rule';
    case 'confirmed':
      return 'Paired by you';
    default:
      return 'Needs your decision';
  }
}

interface Group {
  table: MappingMatch;
  columns: MappingMatch[];
}

/** Group column pairs under their table pair, in name order.  A match that
 *  does not say what it is is shown as a table. */
export function groupMatches(matches: MappingMatch[]): Group[] {
  const tables = new Map<string, Group>();
  const orphans: MappingMatch[] = [];
  for (const m of matches) {
    if (m.kind !== 'column') tables.set(m.group ?? m.id, { table: m, columns: [] });
  }
  for (const m of matches) {
    if (m.kind !== 'column') continue;
    const g = m.group ? tables.get(m.group) : undefined;
    if (g) g.columns.push(m);
    else orphans.push(m);
  }
  const byOld = (a: MappingMatch, b: MappingMatch) => a.oldName.localeCompare(b.oldName);
  const groups = [...tables.values()].sort((a, b) => byOld(a.table, b.table));
  for (const g of groups) g.columns.sort(byOld);
  // A column whose table is not listed still shows, as its own row.
  for (const m of orphans.sort(byOld)) groups.push({ table: m, columns: [] });
  return groups;
}

const label = (m: MappingMatch, side: 'old' | 'new') =>
  (side === 'old' ? m.oldLabel ?? m.oldName : m.newLabel ?? m.newName) || '—';

function Pair({ m, column }: { m: MappingMatch; column?: boolean }) {
  const renamed = label(m, 'old') !== label(m, 'new');
  const waiting = !m.approved;
  return (
    <>
      <span className={cn('min-w-0 break-words', column ? 'font-normal' : 'font-semibold')}>
        {label(m, 'old')}
      </span>
      <ArrowRight className="mx-1 h-3.5 w-3.5 shrink-0 text-slate-400" aria-label="pairs with" />
      <span
        className={cn(
          'min-w-0 break-words',
          column ? 'font-normal' : 'font-semibold',
          renamed && 'text-accent-700 dark:text-accent-300',
        )}
      >
        {label(m, 'new')}
      </span>
      <span
        className={cn(
          'ml-auto shrink-0 rounded-full px-2 py-0.5 text-[11px] font-semibold',
          waiting
            ? 'bg-amber-100 text-amber-900 dark:bg-amber-500/15 dark:text-amber-200'
            : renamed
              ? 'bg-accent-100 text-accent-900 dark:bg-accent-500/20 dark:text-accent-100'
              : 'bg-slate-100 text-slate-700 dark:bg-white/[0.08] dark:text-slate-200',
        )}
      >
        {howMatched(m)}
        {renamed && !waiting && m.confidence !== 'rule' && m.confidence !== 'confirmed'
          ? ' (renamed)'
          : ''}
      </span>
    </>
  );
}

/**
 * Everything the mapping pairs, table by table with each table's columns: the
 * old name, the new name, and how the pair was made.  Read only: changes are
 * made in the list of items that need attention, or with rename rules.
 */
export function MatchedList({ matches }: { matches: MappingMatch[] }) {
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [onlyRenamed, setOnlyRenamed] = useState(false);
  const groups = useMemo(() => groupMatches(matches), [matches]);

  const q = query.trim().toLowerCase();
  const hit = (m: MappingMatch) =>
    !q || `${m.oldName} ${m.newName} ${m.oldLabel ?? ''} ${m.newLabel ?? ''}`.toLowerCase().includes(q);
  const changed = (m: MappingMatch) => label(m, 'old') !== label(m, 'new') || !m.approved;

  const shown = groups
    .map((g) => {
      const cols = g.columns.filter((c) => hit(c) && (!onlyRenamed || changed(c)));
      const selfOk = hit(g.table) && (!onlyRenamed || changed(g.table));
      return { ...g, cols, visible: selfOk || cols.length > 0 };
    })
    .filter((g) => g.visible);

  const tableCount = groups.length;
  const columnCount = groups.reduce((n, g) => n + g.columns.length, 0);

  const toggle = (id: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <div className="flex flex-col gap-3" data-testid="matched-list">
      <p className="text-sm text-slate-600 dark:text-slate-300">
        {tableCount === 1 ? '1 table' : `${tableCount} tables`} and{' '}
        {columnCount === 1 ? '1 column' : `${columnCount} columns`} are paired. Open a table to see
        its columns. Change a pair in the list of items that need attention, or with rename rules.
      </p>
      <div className="flex flex-wrap items-center gap-4">
        <label className="relative min-w-[16rem] flex-1">
          <span className="sr-only">Find a table or column in the mapping</span>
          <Search
            className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-400"
            aria-hidden="true"
          />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Find a table or column"
            className="h-9 w-full rounded-[10px] border border-black/[0.1] bg-white pl-9 pr-3 text-sm text-ink focus:border-accent-500 focus:outline-none focus:ring-4 focus:ring-accent-500/20 dark:border-white/10 dark:bg-white/[0.06] dark:text-slate-100"
          />
        </label>
        <label className="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-200">
          <input
            type="checkbox"
            className="h-4 w-4 accent-accent-600"
            checked={onlyRenamed}
            onChange={(e) => setOnlyRenamed(e.target.checked)}
          />
          Only renamed or undecided
        </label>
      </div>
      {shown.length === 0 ? (
        <p className="text-sm text-slate-600 dark:text-slate-300">Nothing matches.</p>
      ) : (
        <ul className="max-h-[28rem] divide-y divide-black/[0.06] overflow-auto rounded-xl border border-black/[0.08] dark:divide-white/[0.06] dark:border-white/10">
          {shown.map((g) => {
            const id = g.table.group ?? g.table.id;
            const expanded = open.has(id) || (!!q && g.cols.length > 0) || (onlyRenamed && g.cols.length > 0);
            return (
              <li key={id}>
                <div className="flex items-center gap-2 px-3 py-2.5 text-sm text-ink dark:text-slate-100">
                  {g.columns.length > 0 ? (
                    <button
                      type="button"
                      onClick={() => toggle(id)}
                      aria-expanded={expanded}
                      aria-label={`${expanded ? 'Hide' : 'Show'} the columns of ${g.table.oldName}`}
                      className="rounded p-0.5 text-slate-500 hover:bg-black/[0.05] dark:text-slate-400 dark:hover:bg-white/[0.08]"
                    >
                      {expanded ? (
                        <ChevronDown className="h-4 w-4" aria-hidden="true" />
                      ) : (
                        <ChevronRight className="h-4 w-4" aria-hidden="true" />
                      )}
                    </button>
                  ) : (
                    <span className="w-5 shrink-0" aria-hidden="true" />
                  )}
                  <Pair m={g.table} />
                  {g.columns.length > 0 && (
                    <span className="w-24 shrink-0 text-right text-xs text-slate-500 dark:text-slate-400">
                      {g.columns.length === 1 ? '1 column' : `${g.columns.length} columns`}
                    </span>
                  )}
                </div>
                {expanded && (
                  <ul className="border-t border-black/[0.04] bg-black/[0.015] py-1 dark:border-white/[0.05] dark:bg-white/[0.02]">
                    {(q || onlyRenamed ? g.cols : g.columns).map((c) => (
                      <li
                        key={c.id}
                        className="flex items-center gap-2 py-1.5 pl-12 pr-3 text-sm text-slate-700 dark:text-slate-200"
                      >
                        <Pair m={c} column />
                        <span className="w-24 shrink-0" aria-hidden="true" />
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
