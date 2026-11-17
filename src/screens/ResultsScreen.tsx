import { useEffect, useMemo, useRef, useState } from 'react';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { StatusBadge } from '../design/StatusBadge';
import { ErrorCard } from '../design/ErrorCard';
import { DataTable } from '../components/DataTable';
import { TableDetailPanel, WhatWasChecked } from '../components/TableDetailPanel';
import { LEVEL_LABELS, VerdictBanner, qualifiedVerdictLabel } from '../components/VerdictBanner';
import { createColumnHelper, type ColumnDef } from '@tanstack/react-table';
import type { Engine } from '../api/engine';
import type { DrillDownRow, FailureCard, IgnoreItem, Results, TableResult } from '../api/contract';

interface ResultsScreenProps {
  engine: Engine;
  runId: string;
  results: Results | null;
  onResultsChange: (results: Results) => void;
  onNext: () => void;
  onBack: () => void;
}

export function coverageLabel(coverage: Results['coverage']): string {
  return `${coverage.fullValuesTables} of ${coverage.totalTables} tables fully checked, ${coverage.countsTables} counts only, ${coverage.structureTables} structure only, ${coverage.ignoredTables} ignored`;
}

/** Every ignored table/column is listed on the first page with its required reason. */
function IgnoredList({ ignored }: { ignored: IgnoreItem[] }) {
  if (ignored.length === 0) {
    return null;
  }
  return (
    <div className="mt-3">
      <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">Ignored</h3>
      <ul className="mt-1 space-y-1 text-sm text-slate-600 dark:text-slate-300">
        {ignored.map((item) => (
          <li key={item.id}>
            <span className="font-medium">{item.name}</span>
            <span className="ml-2 text-slate-500 dark:text-slate-400">— {item.reason}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Every ticked option is listed, in table.column: option form (H2 / decision 23). */
function OptionsAppliedList({ results }: { results: Results }) {
  if (!results.optionsRecorded) {
    return (
      <div className="mt-3">
        <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">Options applied</h3>
        <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">
          options not recorded (older results file)
        </p>
      </div>
    );
  }
  const lines: string[] = results.tableResults.flatMap((t) =>
    (t.columnNotes ?? [])
      .filter((n) => n.kind === 'option')
      .map((n) => `${t.schemaName}.${t.tableName}.${n.column}: ${n.note}`),
  );
  if (lines.length === 0) {
    return null;
  }
  return (
    <div className="mt-3">
      <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">Options applied</h3>
      <ul className="mt-1 space-y-1 text-sm text-slate-600 dark:text-slate-300">
        {lines.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
    </div>
  );
}

/** Run-level warnings (can-write, superuser, row-level security, ...). */
function WarningsList({ warnings }: { warnings: string[] }) {
  if (warnings.length === 0) {
    return null;
  }
  return (
    <div className="mt-3">
      <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">Warnings</h3>
      <ul className="mt-1 space-y-1 text-sm text-slate-600 dark:text-slate-300">
        {warnings.map((warning) => (
          <li key={warning}>{warning}</li>
        ))}
      </ul>
    </div>
  );
}

/**
 * A drill-down row is a data difference between the old and new database, so
 * every kind renders as a FAIL-class badge. It is never labelled
 * "Changed during check", which means the table was modified WHILE the check
 * ran (PLAN 3.4) — a different condition with its own verdict.
 */
function drillDownStatus(kind: DrillDownRow['kind']): 'missing-in-new' | 'extra-in-new' | 'changed-values' {
  if (kind === 'missing') return 'missing-in-new';
  if (kind === 'extra') return 'extra-in-new';
  return 'changed-values';
}

/**
 * How many drill-down rows render at first, and how many more each "Show more"
 * adds.  The engine caps changed rows (CHANGED_DETAIL_LIMIT) but not missing or
 * extra keys, so one table can carry tens of thousands; rendering them all at
 * once froze the webview.
 */
export const DRILL_DOWN_PAGE = 50;

function DrillDown({ rows }: { rows: DrillDownRow[] }) {
  const [shown, setShown] = useState(DRILL_DOWN_PAGE);
  const visible = rows.slice(0, shown);
  const remaining = rows.length - visible.length;
  return (
    <div className="mt-2 space-y-2">
      {/* Always say how many rows exist, so a partial list never reads as complete. */}
      <p className="text-sm text-slate-600 dark:text-slate-300">
        Showing {visible.length.toLocaleString()} of {rows.length.toLocaleString()} rows
      </p>
      {visible.map((row, idx) => (
        <div
          key={`${row.key}-${idx}`}
          className="rounded-lg border border-slate-200 p-2 text-sm dark:border-slate-700"
        >
          <div className="font-medium text-slate-900 dark:text-slate-100">{row.key}</div>
          <StatusBadge status={drillDownStatus(row.kind)} />
          {row.kind === 'changed' && row.differingColumns && (
            <div className="mt-1 text-slate-600 dark:text-slate-300">
              Changed columns: {row.differingColumns.join(', ')}
            </div>
          )}
        </div>
      ))}
      {remaining > 0 && (
        <Button
          onClick={() => setShown((n) => n + DRILL_DOWN_PAGE)}
          variant="secondary"
          size="sm"
        >
          Show {Math.min(DRILL_DOWN_PAGE, remaining).toLocaleString()} more (
          {remaining.toLocaleString()} not shown)
        </Button>
      )}
    </div>
  );
}

function FailureCardItem({
  card,
  onDrillDown,
  onDetails,
}: {
  card: FailureCard;
  onDrillDown: () => void;
  onDetails: () => void;
}) {
  return (
    <div className="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-700 dark:bg-slate-900">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="font-semibold text-slate-900 dark:text-slate-100">
            {card.schemaName}.{card.tableName}
          </h3>
          <p className="text-sm font-medium text-slate-700 dark:text-slate-300">{card.summary}</p>
          <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">{card.detail}</p>
        </div>
        <StatusBadge status={card.verdict} />
      </div>
      <div className="mt-2 flex flex-wrap gap-2">
        <Button onClick={onDrillDown} variant="ghost" size="sm">
          Show rows
        </Button>
        <Button
          onClick={onDetails}
          variant="ghost"
          size="sm"
          aria-label={`Show details for ${card.schemaName}.${card.tableName}`}
        >
          Show details
        </Button>
      </div>
    </div>
  );
}

export function ResultsScreen({
  engine,
  runId,
  results,
  onResultsChange,
  onNext,
  onBack,
}: ResultsScreenProps) {
  const [error, setError] = useState<string | null>(null);
  const [expandedFailures, setExpandedFailures] = useState<Set<string>>(new Set());
  const [detailId, setDetailId] = useState<string | null>(null);
  const detailHeading = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    let mounted = true;
    async function load() {
      setError(null);
      try {
        const result = await engine.getResults(runId);
        if (mounted) {
          onResultsChange(result);
        }
      } catch (e) {
        if (mounted) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    }
    if (!results) {
      void load();
    }
    return () => {
      mounted = false;
    };
  }, [engine, runId, results, onResultsChange]);

  // Open the detail panel for a table and move focus to its heading.
  useEffect(() => {
    if (detailId !== null) {
      detailHeading.current?.focus();
      detailHeading.current?.scrollIntoView?.({ block: 'nearest' });
    }
  }, [detailId]);

  const tableColumns = useMemo(() => {
    const col = createColumnHelper<TableResult>();
    return [
      col.accessor('schemaName', { header: 'Schema' }),
      col.accessor('tableName', { header: 'Table' }),
      col.accessor('level', {
        header: 'Level',
        cell: ({ getValue }) => LEVEL_LABELS[getValue() as TableResult['level']] ?? getValue(),
      }),
      col.accessor('verdict', {
        header: 'Verdict',
        cell: ({ getValue }) => <StatusBadge status={getValue() as TableResult['verdict']} />,
      }),
      col.display({
        id: 'notes',
        header: 'Notes',
        cell: ({ row }) => {
          const notes = row.original.columnNotes ?? [];
          if (notes.length === 0) {
            return null;
          }
          return (
            <span className="text-xs text-slate-500 dark:text-slate-400">
              {notes.map((n) => `${n.column}: ${n.note}`).join('; ')}
            </span>
          );
        },
      }),
      col.display({
        id: 'details',
        header: 'Details',
        cell: ({ row }) => (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setDetailId(row.original.tableId)}
            aria-label={`Details for ${row.original.schemaName}.${row.original.tableName}`}
          >
            Details
          </Button>
        ),
      }),
    ] as ColumnDef<TableResult>[];
  }, []);

  if (!results) {
    return (
      <div className="space-y-6">
        <ScreenHeader title="Results" subtitle="The verdict comes first, then anything that needs your attention." />
        {error && <ErrorCard title="Could not load results">{error}</ErrorCard>}
        {!error && (
          <Panel>
            <div className="py-8 text-center text-slate-600 dark:text-slate-300">
              Loading results…
            </div>
          </Panel>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <ScreenHeader title="Results" subtitle="The verdict comes first, then anything that needs your attention." />
      {error && <ErrorCard title="Could not load results">{error}</ErrorCard>}
      <Panel>
        <VerdictBanner verdict={results.verdict} label={qualifiedVerdictLabel(results)} />
        <p className="mt-3 text-sm text-slate-700 dark:text-slate-300">
          {coverageLabel(results.coverage)}
        </p>
        <IgnoredList ignored={results.ignored} />
        <OptionsAppliedList results={results} />
        <WarningsList warnings={results.warnings} />
        <p className="mt-3 text-sm text-slate-600 dark:text-slate-300">
          A PASS means every in-scope table passed at its chosen level, nothing was skipped without a
          recorded reason, and no table changed during the check.
        </p>
      </Panel>

      {results.failures.length > 0 && (
        <Panel>
          <h3 className="mb-4 text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">Failures</h3>
          <div className="space-y-4">
            {results.failures.map((card) => {
              const expanded = expandedFailures.has(card.id);
              const related = results.tableResults.find((t) => t.tableId === card.tableId);
              return (
                <div key={card.id}>
                  <FailureCardItem
                    card={card}
                    onDetails={() => setDetailId(card.tableId)}
                    onDrillDown={() =>
                      setExpandedFailures((prev) => {
                        const next = new Set(prev);
                        if (next.has(card.id)) {
                          next.delete(card.id);
                        } else {
                          next.add(card.id);
                        }
                        return next;
                      })
                    }
                  />
                  {expanded && related?.drillDown && <DrillDown rows={related.drillDown} />}
                </div>
              );
            })}
          </div>
        </Panel>
      )}

      {(() => {
        const open = results.tableResults.find((t) => t.tableId === detailId);
        return open ? (
          <TableDetailPanel ref={detailHeading} table={open} onClose={() => setDetailId(null)} />
        ) : null;
      })()}

      <Panel>
        <h3 className="mb-4 text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">All tables</h3>
        <DataTable
          data={results.tableResults}
          columns={tableColumns}
          filterPlaceholder="Filter by table name"
        />
      </Panel>

      <WhatWasChecked results={results} />

      <div className="flex items-center justify-between">
        <Button onClick={onBack} variant="secondary">
          Back
        </Button>
        <Button onClick={onNext} variant="primary" size="lg">
          Report
        </Button>
      </div>
    </div>
  );
}
