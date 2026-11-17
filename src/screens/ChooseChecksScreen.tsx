import { useEffect, useMemo, useState } from 'react';
import { Panel } from '../design/Panel';
import { Button } from '../design/Button';
import { Select } from '../design/Select';
import { DataTable } from '../components/DataTable';
import { ErrorCard } from '../design/ErrorCard';
import { createColumnHelper, type ColumnDef } from '@tanstack/react-table';
import type { Engine } from '../api/engine';
import type { CheckLevel, CheckPlan, Mapping, TableCheck } from '../api/contract';

interface ChooseChecksScreenProps {
  engine: Engine;
  mapping: Mapping;
  plan: CheckPlan | null;
  onPlanChange: (plan: CheckPlan) => void;
  onNext: () => void;
  onBack: () => void;
}

const levelOptions: { value: CheckLevel; label: string }[] = [
  { value: 'structure', label: 'Structure' },
  { value: 'counts', label: 'Counts and totals' },
  { value: 'full-values', label: 'Full values' },
];

export function ChooseChecksScreen({
  engine,
  mapping,
  plan,
  onPlanChange,
  onNext,
  onBack,
}: ChooseChecksScreenProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedPreset, setSelectedPreset] = useState<string>('');

  useEffect(() => {
    let mounted = true;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        const result = await engine.buildCheckPlan(mapping);
        if (mounted) {
          onPlanChange(result);
          setSelectedPreset(result.presets[0] ?? '');
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
    if (!plan) {
      void load();
    }
    return () => {
      mounted = false;
    };
  }, [engine, mapping, plan, onPlanChange]);

  const updateLevel = (tableId: string, level: CheckLevel) => {
    if (!plan) return;
    onPlanChange({
      ...plan,
      tables: plan.tables.map((t) => (t.tableId === tableId ? { ...t, level } : t)),
    });
  };

  const columns = useMemo(() => {
    const col = createColumnHelper<TableCheck>();
    return [
      col.accessor('schemaName', { header: 'Schema' }),
      col.accessor('tableName', { header: 'Table' }),
      col.accessor('level', {
        header: 'Check level',
        cell: ({ row }) => (
          <Select
            options={levelOptions}
            value={row.original.level}
            onChange={(e) => updateLevel(row.original.tableId, e.target.value as CheckLevel)}
            className="w-48"
          />
        ),
      }),
      col.accessor('controlTotalColumns', {
        header: 'Control totals',
        cell: ({ getValue }) => (getValue() as string[] | undefined)?.join(', ') ?? '—',
      }),
    ] as ColumnDef<TableCheck>[];
  }, [plan]);

  const totals = useMemo(() => {
    if (!plan) return null;
    const full = plan.tables.filter((t) => t.level === 'full-values').length;
    const counts = plan.tables.filter((t) => t.level === 'counts').length;
    const structure = plan.tables.filter((t) => t.level === 'structure').length;
    return { full, counts, structure };
  }, [plan]);

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">Choose checks</h2>
      {loading && (
        <Panel>
          <div className="py-8 text-center text-slate-600 dark:text-slate-300">Building check plan…</div>
        </Panel>
      )}
      {error && <ErrorCard title="Could not build check plan">{error}</ErrorCard>}
      {plan && (
        <>
          <Panel className="flex flex-col gap-4">
            <div className="flex flex-wrap items-end gap-4">
              <Select
                label="Preset"
                options={plan.presets.map((p) => ({ value: p, label: p }))}
                value={selectedPreset}
                onChange={(e) => setSelectedPreset(e.target.value)}
                className="w-72"
              />
              <div className="rounded-lg border border-slate-200 px-3 py-2 text-sm dark:border-slate-700">
                <span className="font-medium">Estimate:</span>{' '}
                {plan.estimateSecondsMin}s – {plan.estimateSecondsMax}s
              </div>
            </div>
            {totals && (
              <div className="flex gap-4 text-sm text-slate-600 dark:text-slate-300">
                <span><strong>{totals.full}</strong> full values</span>
                <span><strong>{totals.counts}</strong> counts</span>
                <span><strong>{totals.structure}</strong> structure only</span>
              </div>
            )}
            <DataTable
              data={plan.tables}
              columns={columns}
              filterPlaceholder="Filter by schema or table name"
            />
          </Panel>
          <div className="flex justify-between">
            <Button onClick={onBack} variant="secondary">
              Back
            </Button>
            <Button onClick={onNext} variant="primary" size="lg">
              Run
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
