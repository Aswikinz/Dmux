import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { ChevronDown, ChevronRight, Search } from 'lucide-react';
import { cn } from '../design/cn';
import { StatusBadge } from '../design/StatusBadge';
import type { StructureRow } from '../api/contract';
import { KIND_GROUP_LABELS, visibleRows, type KindGroup, type VisibleRow } from './structureRows';

/** One line is this tall, so scrolling maths never needs to measure. */
export const STRUCTURE_ROW_HEIGHT = 36;
const HEADER_HEIGHT = 52;
const GRID = 'grid grid-cols-[minmax(200px,1.2fr)_minmax(0,1fr)_132px_minmax(0,1fr)] items-stretch';

interface SideLabel {
  title: string;
  detail: string;
}

interface StructureCompareViewProps {
  rows: StructureRow[];
  old: SideLabel;
  new: SideLabel;
  showOnlyProblems: boolean;
}

const GAP_STYLE =
  'bg-[repeating-linear-gradient(135deg,transparent_0,transparent_6px,rgba(0,0,0,0.06)_6px,rgba(0,0,0,0.06)_8px)] dark:bg-[repeating-linear-gradient(135deg,transparent_0,transparent_6px,rgba(255,255,255,0.08)_6px,rgba(255,255,255,0.08)_8px)]';

function cellTone(status: StructureRow['status'], side: 'old' | 'new'): string {
  switch (status) {
    case 'missing':
      return side === 'old'
        ? 'bg-red-50 text-red-900 dark:bg-red-500/10 dark:text-red-100'
        : cn('text-red-800 dark:text-red-200', GAP_STYLE);
    case 'added':
      return side === 'new'
        ? 'bg-emerald-50 text-emerald-900 dark:bg-emerald-500/10 dark:text-emerald-100'
        : cn('text-emerald-800 dark:text-emerald-200', GAP_STYLE);
    case 'changed':
      return 'bg-amber-50 text-amber-900 dark:bg-amber-500/10 dark:text-amber-100';
    default:
      return 'text-slate-700 dark:text-slate-300';
  }
}

function Cell({ row, side }: { row: VisibleRow; side: 'old' | 'new' }) {
  const text = row[side];
  const sideWord = side === 'old' ? 'old' : 'new';
  return (
    <div
      role="gridcell"
      data-gap={text === undefined ? 'true' : undefined}
      data-side={side}
      className={cn('flex min-w-0 items-center px-3 text-sm', cellTone(row.status, side))}
    >
      {text === undefined ? (
        <span className="text-xs font-medium italic">
          {`Not in the ${sideWord} database`}
        </span>
      ) : (
        <span className="truncate font-mono text-[13px]" title={text}>
          {text}
        </span>
      )}
    </div>
  );
}

/**
 * The old database on the left and the new one on the right, aligned so the
 * same table, column, index or constraint sits on the same line.  A gap marks
 * what one side lacks; the status badge says it in words, so colour is never
 * the only signal.  The list is virtualised, so thousands of tables stay
 * smooth, and it is a keyboard-operable tree grid.
 */
export function StructureCompareView({
  rows,
  old,
  new: next,
  showOnlyProblems,
}: StructureCompareViewProps) {
  const uid = useId();
  const [query, setQuery] = useState('');
  const [toggled, setToggled] = useState<ReadonlySet<string>>(new Set());
  const [activeId, setActiveId] = useState<string | null>(null);
  const [hiddenGroups, setHiddenGroups] = useState<ReadonlySet<KindGroup>>(new Set());
  const scrollRef = useRef<HTMLDivElement>(null);

  const shown = useMemo(
    () => visibleRows(rows, { toggled, onlyProblems: showOnlyProblems, query, hiddenGroups }),
    [rows, toggled, showOnlyProblems, query, hiddenGroups],
  );
  const list = shown.rows;
  const activeIndex = Math.max(
    0,
    list.findIndex((r) => r.id === activeId),
  );

  const virtualizer = useVirtualizer({
    count: list.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => STRUCTURE_ROW_HEIGHT,
    overscan: 12,
    initialRect: { width: 900, height: 520 },
  });

  // Keep the active line in view while moving by keyboard.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || activeId === null) return;
    const top = activeIndex * STRUCTURE_ROW_HEIGHT;
    const visible = el.clientHeight - HEADER_HEIGHT;
    if (top < el.scrollTop) {
      el.scrollTop = top;
    } else if (visible > 0 && top + STRUCTURE_ROW_HEIGHT > el.scrollTop + visible) {
      el.scrollTop = top + STRUCTURE_ROW_HEIGHT - visible;
    }
  }, [activeId, activeIndex]);

  const toggle = (id: string) =>
    setToggled((prev) => {
      const nextSet = new Set(prev);
      if (nextSet.has(id)) {
        nextSet.delete(id);
      } else {
        nextSet.add(id);
      }
      return nextSet;
    });

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target instanceof HTMLInputElement || list.length === 0) return;
    const row = list[activeIndex];
    const move = (to: number) => {
      const clamped = Math.min(list.length - 1, Math.max(0, to));
      setActiveId(list[clamped].id);
      e.preventDefault();
    };
    switch (e.key) {
      case 'ArrowDown':
        move(activeIndex + 1);
        break;
      case 'ArrowUp':
        move(activeIndex - 1);
        break;
      case 'PageDown':
        move(activeIndex + 10);
        break;
      case 'PageUp':
        move(activeIndex - 10);
        break;
      case 'Home':
        move(0);
        break;
      case 'End':
        move(list.length - 1);
        break;
      case 'ArrowRight':
        if (row.expandable && !row.expanded) toggle(row.id);
        e.preventDefault();
        break;
      case 'ArrowLeft':
        if (row.expandable && row.expanded) toggle(row.id);
        e.preventDefault();
        break;
      case 'Enter':
      case ' ':
        if (row.expandable) toggle(row.id);
        e.preventDefault();
        break;
      default:
    }
  };

  const problemWord = shown.differences === 1 ? 'difference' : 'differences';
  const tablesOnly = hiddenGroups.size === 2 && !hiddenGroups.has('tables');
  const activeDomId = list.length > 0 ? `${uid}-row-${activeIndex}` : undefined;

  return (
    <div className="flex flex-col gap-3" data-testid="structure-compare">
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative min-w-[16rem] flex-1">
          <Search
            className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500 dark:text-slate-400"
            aria-hidden="true"
          />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search tables, columns, indexes"
            aria-label="Search the structure"
            className="h-9 w-full rounded-[10px] border border-black/[0.1] bg-black/[0.03] pl-9 pr-3 text-sm placeholder:text-slate-500 focus:border-accent-500 focus:bg-white focus:outline-none focus:ring-4 focus:ring-accent-500/20 dark:border-white/10 dark:bg-white/[0.06] dark:text-slate-100 dark:placeholder:text-slate-400"
          />
        </div>
        <p className="text-sm text-slate-700 dark:text-slate-300" role="status" data-testid="structure-count">
          {shown.differences.toLocaleString()} {problemWord} · showing {list.length.toLocaleString()} of{' '}
          {shown.total.toLocaleString()} items
          {shown.hiddenDifferences > 0 &&
            ` · ${shown.hiddenDifferences.toLocaleString()} more in the kinds you hid`}
        </p>
      </div>
      <fieldset className="flex flex-wrap items-center gap-x-4 gap-y-2" data-testid="structure-kinds">
        <legend className="float-left mr-1 text-sm font-medium text-slate-700 dark:text-slate-200">
          Show:
        </legend>
        {(Object.keys(KIND_GROUP_LABELS) as KindGroup[]).map((g) => (
          <label key={g} className="flex items-center gap-1.5 text-sm text-slate-700 dark:text-slate-200">
            <input
              type="checkbox"
              className="h-4 w-4 accent-accent-600"
              checked={!hiddenGroups.has(g)}
              onChange={(e) => {
                const next = new Set(hiddenGroups);
                if (e.target.checked) next.delete(g);
                else next.add(g);
                setHiddenGroups(next);
              }}
            />
            {KIND_GROUP_LABELS[g]}
          </label>
        ))}
        <button
          type="button"
          className="rounded-lg border border-black/[0.1] px-2.5 py-1 text-[13px] font-medium text-slate-700 hover:bg-black/[0.04] dark:border-white/10 dark:text-slate-200 dark:hover:bg-white/[0.06]"
          onClick={() => setHiddenGroups(tablesOnly ? new Set() : new Set<KindGroup>(['indexes', 'other']))}
        >
          {tablesOnly ? 'Show every kind' : 'Tables and columns only'}
        </button>
      </fieldset>

      <div
        ref={scrollRef}
        role="treegrid"
        aria-label="Structure of the old and the new database, side by side"
        aria-rowcount={list.length + 1}
        aria-activedescendant={activeDomId}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onFocus={() => {
          if (activeId === null && list.length > 0) setActiveId(list[0].id);
        }}
        className="max-h-[62vh] min-h-[16rem] overflow-auto rounded-xl border border-black/[0.06] bg-white focus:outline-none focus-visible:ring-4 focus-visible:ring-accent-500/30 dark:border-white/[0.08] dark:bg-card-dark"
      >
        <div className="min-w-[820px]">
          <div
            role="row"
            aria-rowindex={1}
            className={cn(
              GRID,
              'sticky top-0 z-10 border-b border-black/[0.08] bg-slate-50 dark:border-white/[0.1] dark:bg-[#2a2a2d]',
            )}
            style={{ height: HEADER_HEIGHT }}
          >
            <div role="columnheader" className="flex items-center px-3 text-[11px] font-semibold uppercase tracking-[0.05em] text-ink-secondary dark:text-slate-400">
              Item
            </div>
            <div role="columnheader" className="flex min-w-0 flex-col justify-center px-3" data-testid="old-header">
              <span className="text-sm font-semibold text-slate-900 dark:text-slate-100">{old.title}</span>
              <span className="truncate text-xs text-slate-600 dark:text-slate-300">{old.detail}</span>
            </div>
            <div role="columnheader" className="flex items-center justify-center px-2 text-[11px] font-semibold uppercase tracking-[0.05em] text-ink-secondary dark:text-slate-400">
              Result
            </div>
            <div role="columnheader" className="flex min-w-0 flex-col justify-center px-3" data-testid="new-header">
              <span className="text-sm font-semibold text-slate-900 dark:text-slate-100">{next.title}</span>
              <span className="truncate text-xs text-slate-600 dark:text-slate-300">{next.detail}</span>
            </div>
          </div>

          {list.length === 0 ? (
            <p className="p-6 text-center text-sm text-slate-700 dark:text-slate-300" role="row">
              <span role="gridcell">Nothing matches. Clear the search or turn off &ldquo;Show only problems&rdquo;.</span>
            </p>
          ) : (
            <div style={{ position: 'relative', height: virtualizer.getTotalSize() }}>
              {virtualizer.getVirtualItems().map((v) => {
                const row = list[v.index];
                if (!row) return null;
                const active = v.index === activeIndex;
                return (
                  <div
                    key={row.id}
                    id={`${uid}-row-${v.index}`}
                    role="row"
                    // Focus stays on the grid (aria-activedescendant); a row
                    // can still take focus from script, never from Tab.
                    tabIndex={-1}
                    aria-rowindex={v.index + 2}
                    aria-level={row.depth + 1}
                    aria-expanded={row.expandable ? row.expanded : undefined}
                    data-status={row.status}
                    data-testid="structure-row"
                    onClick={() => setActiveId(row.id)}
                    className={cn(
                      GRID,
                      'absolute left-0 w-full border-b border-black/[0.04] dark:border-white/[0.05]',
                      active && 'outline outline-2 -outline-offset-2 outline-accent-500',
                    )}
                    style={{ top: 0, height: STRUCTURE_ROW_HEIGHT, transform: `translateY(${v.start}px)` }}
                  >
                    <div
                      role="rowheader"
                      className="flex min-w-0 items-center gap-1.5 pr-2"
                      style={{ paddingLeft: row.depth * 18 + 8 }}
                    >
                      {row.expandable ? (
                        <button
                          type="button"
                          tabIndex={-1}
                          onClick={(e) => {
                            e.stopPropagation();
                            toggle(row.id);
                          }}
                          aria-label={`${row.expanded ? 'Collapse' : 'Expand'} ${row.name}`}
                          className="rounded-md p-0.5 text-slate-600 hover:bg-black/[0.06] dark:text-slate-300 dark:hover:bg-white/10"
                        >
                          {row.expanded ? (
                            <ChevronDown className="h-4 w-4" aria-hidden="true" />
                          ) : (
                            <ChevronRight className="h-4 w-4" aria-hidden="true" />
                          )}
                        </button>
                      ) : (
                        <span className="inline-block h-4 w-5 shrink-0" aria-hidden="true" />
                      )}
                      <span className="truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={row.name}>
                        {row.name}
                      </span>
                      <span className="shrink-0 text-[11px] uppercase tracking-[0.04em] text-slate-600 dark:text-slate-400">
                        {row.kind}
                      </span>
                      {row.expandable && !row.expanded && row.problemsBelow > 0 && (
                        <span className="shrink-0 text-xs text-amber-800 dark:text-amber-300">
                          {row.problemsBelow} {row.problemsBelow === 1 ? 'difference' : 'differences'}
                        </span>
                      )}
                    </div>
                    <Cell row={row} side="old" />
                    <div role="gridcell" className="flex items-center justify-center px-2">
                      {row.status !== 'same' && (
                        <>
                          <StatusBadge status={row.status} />
                          {row.status === 'changed' && row.old !== undefined && row.new !== undefined && (
                            <span className="sr-only" data-testid="diff-before-after">
                              changed from {row.old} → {row.new}
                            </span>
                          )}
                        </>
                      )}
                    </div>
                    <Cell row={row} side="new" />
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
      <p className="text-xs text-slate-600 dark:text-slate-400">
        Use the arrow keys to move, Right and Left to open and close a group, Enter to toggle.
        You can also click the arrow beside a name.
      </p>
    </div>
  );
}
