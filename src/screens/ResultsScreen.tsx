import { useEffect, useMemo, useState } from 'react';
import { Panel } from '../design/Panel';
import { Button } from '../design/Button';
import { StatusBadge } from '../design/StatusBadge';
import { ErrorCard } from '../design/ErrorCard';
import { DataTable } from '../components/DataTable';
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

function coverageLabel(coverage: Results['coverage']): string {
  return `${coverage.fullValuesTables} of ${coverage.totalTables} tables fully checked, ${coverage.countsTables} counts only, ${coverage.structureTables} structure only, ${coverage.ignoredTables} ignored`;
}

/** Every ignored table/column is listed on the first page with its required reason. */
function IgnoredList({ ignored }: { ignored: IgnoreItem[] }) {
  if (ignored.length === 0) {
    return null;
  }
  return (
    <div className="mt-3">
      <h3 className="text-sm font-semibold text-slate-700 dark:text-slate-300">Ignored</h3>
      <ul className="mt-1 space-y-1 text-sm text-slate-600 dark:text-slate-300">
        {ignored.map((item) => (
          <li key={item.id}>
            <span className="font-medium">{item.name}</span>
            <span className="ml-2 text-slate-500">— {item.reason}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function DrillDown({ rows }: { rows: DrillDownRow[] }) {
  return (
    <div className="mt-2 space-y-2">
      {rows.map((row, idx) => (
        <div
          key={`${row.key}-${idx}`}
          className="rounded-lg border border-slate-200 p-2 text-sm dark:border-slate-700"
        >
          <div className="font-medium text-slate-900 dark:text-slate-100">{row.key}</div>
          <StatusBadge status={row.kind === 'missing' ? 'FAIL' : row.kind === 'extra' ? 'FAIL' : 'CHANGED_DURING_CHECK'} />
          {row.kind === 'changed' && row.differingColumns && (
            <div className="mt-1 text-slate-600 dark:text-slate-300">
              Changed columns: {row.differingColumns.join(', ')}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function FailureCardItem({ card, onDrillDown }: { card: FailureCard; onDrillDown: () => void }) {
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
      <Button onClick={onDrillDown} variant="ghost" size="sm" className="mt-2">
        Show rows
      </Button>
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
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedFailures, setExpandedFailures] = useState<Set<string>>(new Set());

  useEffect(() => {
    let mounted = true;
    async function load() {
      setLoading(true);
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
      } finally {
        if (mounted) {
          setLoading(false);
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

  const tableColumns = useMemo(() => {
    const col = createColumnHelper<TableResult>();
    return [
      col.accessor('schemaName', { header: 'Schema' }),
      col.accessor('tableName', { header: 'Table' }),
      col.accessor('level', { header: 'Level' }),
      col.accessor('verdict', {
        header: 'Verdict',
        cell: ({ getValue }) => <StatusBadge status={getValue() as TableResult['verdict']} />,
      }),
    ] as ColumnDef<TableResult>[];
  }, []);

  if (loading || !results) {
    return (
      <div className="space-y-6">
        <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">Results</h2>
        <Panel>
          <div className="py-8 text-center text-slate-600 dark:text-slate-300">Loading results…</div>
        </Panel>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">Results</h2>
      {error && <ErrorCard title="Could not load results">{error}</ErrorCard>}
      <Panel>
        <div className="flex flex-wrap items-center gap-4">
          <StatusBadge status={results.verdict} className="text-base" />
          <p className="text-sm text-slate-700 dark:text-slate-300">{coverageLabel(results.coverage)}</p>
        </div>
        <IgnoredList ignored={results.ignored} />
        <p className="mt-3 text-sm text-slate-600 dark:text-slate-300">
          A PASS means every in-scope table passed at its chosen level, nothing was skipped without a
          recorded reason, and no table changed during the check.
        </p>
      </Panel>

      {results.failures.length > 0 && (
        <Panel>
          <h3 className="mb-4 text-lg font-semibold text-slate-900 dark:text-slate-100">Failures</h3>
          <div className="space-y-4">
            {results.failures.map((card) => {
              const expanded = expandedFailures.has(card.id);
              const related = results.tableResults.find((t) => t.tableId === card.tableId);
              return (
                <div key={card.id}>
                  <FailureCardItem
                    card={card}
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

      <Panel>
        <h3 className="mb-4 text-lg font-semibold text-slate-900 dark:text-slate-100">All tables</h3>
        <DataTable
          data={results.tableResults}
          columns={tableColumns}
          filterPlaceholder="Filter by table name"
        />
      </Panel>

      <div className="flex justify-between">
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
