import { useEffect, useMemo, useRef, useState } from 'react';
import { describeError } from '../api/tauriEngine';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { StatusBadge } from '../design/StatusBadge';
import { ErrorCard } from '../design/ErrorCard';
import { DataTable } from '../components/DataTable';
import { TableDetailPanel, WhatWasChecked } from '../components/TableDetailPanel';
import { CustomChecksPanel } from '../components/CustomChecksPanel';
import { IgnoredItems } from '../components/IgnoredItems';
import { VerdictFilter } from '../components/VerdictFilter';
import { LEVEL_LABELS, VerdictBanner, qualifiedVerdictLabel } from '../components/VerdictBanner';
import { createColumnHelper, type ColumnDef } from '@tanstack/react-table';
import type { Engine } from '../api/engine';
import { ISSUE_KIND_LABELS } from '../api/contract';
import type { DrillDownRow, FailureCard, IssueKind, Results, TableResult, Verdict } from '../api/contract';

interface ResultsScreenProps {
  engine: Engine;
  runId: string;
  results: Results | null;
  onResultsChange: (results: Results) => void;
  onNext: () => void;
  onBack: () => void;
}

export function coverageLabel(coverage: Results['coverage']): string {
  const line = `${coverage.fullValuesTables} of ${coverage.totalTables} tables fully checked, ${coverage.countsTables} counts only, ${coverage.structureTables} structure only, ${coverage.ignoredTables} ignored`;
  // Tables in scope that reached no conclusion are named, so the parts always
  // add up to the tables in scope.
  const notChecked = Math.max(
    0,
    coverage.totalTables -
      coverage.fullValuesTables -
      coverage.countsTables -
      coverage.structureTables -
      coverage.ignoredTables,
  );
  return notChecked > 0 ? `${line}, ${notChecked} not checked` : line;
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

/**
 * The table a failure card belongs to.  A table card names the table itself;
 * a structure finding names a table ("s.t") or something inside one
 * ("s.t.column", "s.t.index").  Database-wide findings (encoding, extensions,
 * functions) and sequences belong to no table, so they have no details.
 */
export function owningTable(card: FailureCard, tables: TableResult[]): TableResult | undefined {
  return (
    tables.find((t) => t.tableId === card.tableId) ??
    tables
      .filter((t) => card.tableId.startsWith(`${t.tableId}.`))
      .sort((a, b) => b.tableId.length - a.tableId.length)[0]
  );
}

function FailureCardItem({
  card,
  rowsShown,
  detailsShown,
  onDrillDown,
  onDetails,
}: {
  card: FailureCard;
  /** Whether "Show rows" is offered (only when there are rows to show) and its state. */
  rowsShown: boolean | null;
  /** Whether "Show details" is offered (only when the card belongs to a table) and its state. */
  detailsShown: boolean | null;
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
          <p
            className={
              (card.kind ?? 'other') === 'data'
                ? 'text-[11px] font-semibold uppercase tracking-[0.06em] text-accent-700 dark:text-accent-300'
                : 'text-[11px] font-semibold uppercase tracking-[0.06em] text-slate-500 dark:text-slate-400'
            }
            data-testid="card-kind"
          >
            {ISSUE_KIND_LABELS[card.kind ?? 'other']}
          </p>
          <p className="text-sm font-medium text-slate-700 dark:text-slate-300">{card.summary}</p>
          <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">{card.detail}</p>
        </div>
        <StatusBadge status={card.verdict} />
      </div>
      {(rowsShown !== null || detailsShown !== null) && (
        <div className="mt-2 flex flex-wrap gap-2">
          {rowsShown !== null && (
            <Button onClick={onDrillDown} variant="ghost" size="sm" aria-expanded={rowsShown}>
              {rowsShown ? 'Hide rows' : 'Show rows'}
            </Button>
          )}
          {detailsShown !== null && (
            <Button
              onClick={onDetails}
              variant="ghost"
              size="sm"
              aria-expanded={detailsShown}
              aria-label={`${detailsShown ? 'Hide' : 'Show'} details for ${card.schemaName}.${card.tableName}`}
            >
              {detailsShown ? 'Hide details' : 'Show details'}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

/** One result: a table row, or a failure card with no row of its own.  A
 *  card and the row of the table it belongs to are one result, so nothing is
 *  counted twice. */
interface ResultItem {
  verdict: Verdict;
  kind?: IssueKind;
}

export function resultItems(results: Results): ResultItem[] {
  const byKey = new Map<string, ResultItem>();
  // Tables that could not be paired are listed in their own panel.
  const unmapped = new Set(
    results.tableResults.filter((t) => t.kind === 'not_mapped').map((t) => t.tableId),
  );
  for (const t of results.tableResults) {
    if (unmapped.has(t.tableId)) continue;
    byKey.set(`t:${t.tableId}`, { verdict: t.verdict, kind: t.kind });
  }
  for (const c of results.failures) {
    if (c.kind === 'not_mapped' || unmapped.has(c.tableId)) continue;
    const row = byKey.get(`t:${c.tableId}`);
    if (row && row.verdict === c.verdict) {
      row.kind ??= c.kind;
    } else {
      byKey.set(`c:${c.id}`, { verdict: c.verdict, kind: c.kind ?? 'other' });
    }
  }
  return [...byKey.values()];
}

/** How many results carry each verdict. */
export function resultCounts(results: Results): Partial<Record<Verdict, number>> {
  const counts: Partial<Record<Verdict, number>> = {};
  for (const { verdict } of resultItems(results)) counts[verdict] = (counts[verdict] ?? 0) + 1;
  return counts;
}

/** How many failures are of each kind (a PASS has no kind). */
export function kindCounts(results: Results): Partial<Record<IssueKind, number>> {
  const counts: Partial<Record<IssueKind, number>> = {};
  for (const { kind } of resultItems(results)) {
    if (kind) counts[kind] = (counts[kind] ?? 0) + 1;
  }
  return counts;
}

/** Whether a result is hidden by the verdict or kind filter. */
const isHidden = (
  item: { verdict: Verdict; kind?: IssueKind },
  verdicts: Set<Verdict>,
  kinds: Set<IssueKind>,
) => verdicts.has(item.verdict) || (item.kind !== undefined && kinds.has(item.kind));

const KIND_RANK: Record<IssueKind, number> = { data: 0, not_mapped: 1, structure: 2, other: 3 };

/** Tables that could not be paired, listed apart from everything compared.
 *  A table missing from the new database fails the check. */
function NotMappedPanel({ results }: { results: Results }) {
  const rows = results.tableResults.filter((t) => t.kind === 'not_mapped');
  if (rows.length === 0) return null;
  return (
    <Panel>
      <h3 className="mb-1 text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">
        Not mapped
      </h3>
      <p className="mb-3 text-sm text-slate-600 dark:text-slate-300">
        These tables have no partner in the other database, so nothing could be compared. A table
        missing from the new database fails the check.
      </p>
      <ul className="divide-y divide-black/[0.06] rounded-xl border border-black/[0.08] dark:divide-white/[0.06] dark:border-white/10" data-testid="not-mapped">
        {rows.map((t) => (
          <li key={t.tableId} className="flex flex-wrap items-center gap-3 px-4 py-2.5 text-sm">
            <span className="font-semibold text-ink dark:text-slate-100">
              {t.schemaName}.{t.tableName}
            </span>
            <span className="text-slate-600 dark:text-slate-300">{t.reasons?.[0] ?? ''}</span>
            <StatusBadge status={t.verdict} className="ml-auto" />
          </li>
        ))}
      </ul>
    </Panel>
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
  // Where the open detail panel sits: under the failure card that opened it,
  // or (null) above the list of all tables.
  const [detailCard, setDetailCard] = useState<string | null>(null);
  const detailHeading = useRef<HTMLHeadingElement>(null);
  // Verdicts the user chose to hide from the lists below (never the banner).
  const [hiddenVerdicts, setHiddenVerdicts] = useState<Set<Verdict>>(new Set());
  const [hiddenKinds, setHiddenKinds] = useState<Set<IssueKind>>(new Set());

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
          setError(describeError(e));
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
  }, [detailId, detailCard]);

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
            onClick={() => {
              setDetailCard(null);
              setDetailId(row.original.tableId);
            }}
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

  // Failures among the tables that were compared; not-mapped tables have
  // their own panel.
  const comparedFailures = results.failures.filter((c) => c.kind !== 'not_mapped');

  return (
    <div className="space-y-6">
      <ScreenHeader title="Results" subtitle="The verdict comes first, then anything that needs your attention." />
      {error && <ErrorCard title="Could not load results">{error}</ErrorCard>}
      <Panel>
        <VerdictBanner verdict={results.verdict} label={qualifiedVerdictLabel(results)} />
        <p className="mt-3 text-sm text-slate-700 dark:text-slate-300">
          {coverageLabel(results.coverage)}
        </p>
        <IgnoredItems ignored={results.ignored} />
        <OptionsAppliedList results={results} />
        <WarningsList warnings={results.warnings} />
        <p className="mt-3 text-sm text-slate-600 dark:text-slate-300">
          A PASS means every in-scope table passed at its chosen level, nothing was skipped without a
          recorded reason, and no table changed during the check.
        </p>
      </Panel>

      <NotMappedPanel results={results} />

      {results.customChecks && <CustomChecksPanel checks={results.customChecks} />}

      {(() => {
        const counts = resultCounts(results);
        const kinds = kindCounts(results);
        const hiddenTotal = resultItems(results).filter((i) =>
          isHidden(i, hiddenVerdicts, hiddenKinds),
        ).length;
        return Object.keys(counts).length > 1 || Object.keys(kinds).length > 1 ? (
          <Panel>
            <VerdictFilter
              counts={counts}
              hidden={hiddenVerdicts}
              onChange={setHiddenVerdicts}
              kindCounts={kinds}
              hiddenKinds={hiddenKinds}
              onKindsChange={setHiddenKinds}
              hiddenTotal={hiddenTotal}
            />
          </Panel>
        ) : null;
      })()}

      {comparedFailures.length > 0 && (
        <Panel>
          <h3 className="mb-4 text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">Failures</h3>
          {comparedFailures.every((card) =>
            isHidden({ ...card, kind: card.kind ?? 'other' }, hiddenVerdicts, hiddenKinds),
          ) && (
            <p className="text-sm text-slate-600 dark:text-slate-300" data-testid="failures-all-hidden">
              All {comparedFailures.length === 1 ? 'of them is' : `${comparedFailures.length} of them are`}{' '}
              hidden by the filter above.
            </p>
          )}
          <div className="space-y-4">
            {results.failures
              .filter((card) => card.kind !== 'not_mapped')
              .filter(
                (card) => !isHidden({ ...card, kind: card.kind ?? 'other' }, hiddenVerdicts, hiddenKinds),
              )
              // Data differences first: they are what a sign-off is about.
              .map((card, index) => ({ card, index }))
              .sort(
                (a, b) =>
                  KIND_RANK[a.card.kind ?? 'other'] - KIND_RANK[b.card.kind ?? 'other'] ||
                  a.index - b.index,
              )
              .map(({ card }) => {
              const expanded = expandedFailures.has(card.id);
              const related = owningTable(card, results.tableResults);
              const hasRows = card.tableId === related?.tableId && (related.drillDown?.length ?? 0) > 0;
              const detailsOpen = related !== undefined && detailCard === card.id && detailId === related.tableId;
              return (
                <div key={card.id}>
                  <FailureCardItem
                    card={card}
                    rowsShown={hasRows ? expanded : null}
                    detailsShown={related ? detailsOpen : null}
                    onDetails={() => {
                      if (!related) return;
                      if (detailsOpen) {
                        setDetailId(null);
                        setDetailCard(null);
                      } else {
                        setDetailCard(card.id);
                        setDetailId(related.tableId);
                      }
                    }}
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
                  {hasRows && expanded && related?.drillDown && <DrillDown rows={related.drillDown} />}
                  {detailsOpen && related && (
                    <div className="mt-3">
                      <TableDetailPanel
                        ref={detailHeading}
                        table={related}
                        onClose={() => {
                          setDetailId(null);
                          setDetailCard(null);
                        }}
                      />
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </Panel>
      )}

      {(() => {
        const open =
          detailCard === null ? results.tableResults.find((t) => t.tableId === detailId) : undefined;
        return open ? (
          <TableDetailPanel ref={detailHeading} table={open} onClose={() => setDetailId(null)} />
        ) : null;
      })()}

      <Panel>
        <h3 className="mb-4 text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">All tables</h3>
        <DataTable
          data={results.tableResults.filter(
            (t) => t.kind !== 'not_mapped' && !isHidden(t, hiddenVerdicts, hiddenKinds),
          )}
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
