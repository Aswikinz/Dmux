import { useEffect, useMemo, useRef, useState } from 'react';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { Select } from '../design/Select';
import { DataTable } from '../components/DataTable';
import { ErrorCard } from '../design/ErrorCard';
import { Tooltip } from '../design/Tooltip';
import { formatDurationRange } from '../components/duration';
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
  { value: 'structure', label: 'Structure only' },
  { value: 'counts', label: 'Counts and totals' },
  { value: 'full-values', label: 'Full values' },
];

/** The presets the user can pick.  Each one really reassigns table levels. */
export type PresetId = 'suggested' | 'full' | 'counts' | 'structure' | 'custom';

export const PRESETS: { value: PresetId; label: string }[] = [
  { value: 'suggested', label: 'Suggested levels' },
  { value: 'full', label: 'Full values for every table' },
  { value: 'counts', label: 'Counts and totals for every table (then pick tables for full values)' },
  { value: 'structure', label: 'Structure only for every table' },
  { value: 'custom', label: 'Custom (your own choices)' },
];

/** The level a preset gives every table, or the suggested level. */
export function applyPreset(
  plan: CheckPlan,
  preset: PresetId,
  suggested: Record<string, CheckLevel>,
): CheckPlan {
  if (preset === 'custom') return plan;
  const levelFor = (t: TableCheck): CheckLevel => {
    switch (preset) {
      case 'full':
        return 'full-values';
      case 'counts':
        return 'counts';
      case 'structure':
        return 'structure';
      default:
        return suggested[t.tableId] ?? t.level;
    }
  };
  return { ...plan, tables: plan.tables.map((t) => ({ ...t, level: levelFor(t) })) };
}

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
  const [selectedPreset, setSelectedPreset] = useState<PresetId>('suggested');
  const [presetNote, setPresetNote] = useState<string | null>(null);
  // The levels the engine suggested, captured once, so "Suggested levels"
  // can always restore them.
  const suggestedRef = useRef<Record<string, CheckLevel> | null>(null);
  if (plan && !suggestedRef.current) {
    suggestedRef.current = Object.fromEntries(plan.tables.map((t) => [t.tableId, t.level]));
  }

  useEffect(() => {
    let mounted = true;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        const result = await engine.buildCheckPlan(mapping);
        if (mounted) {
          onPlanChange(result);
          setSelectedPreset('suggested');
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

  const choosePreset = (preset: PresetId) => {
    setSelectedPreset(preset);
    if (!plan || preset === 'custom') {
      setPresetNote(null);
      return;
    }
    const next = applyPreset(plan, preset, suggestedRef.current ?? {});
    onPlanChange(next);
    const label = PRESETS.find((p) => p.value === preset)?.label ?? preset;
    setPresetNote(
      preset === 'counts'
        ? `Applied "${label}": ${next.tables.length} tables set to Counts and totals. Now set Full values on the tables you want checked in full.`
        : `Applied "${label}" to ${next.tables.length} tables.`,
    );
  };

  const updateLevel = (tableId: string, level: CheckLevel) => {
    if (!plan) return;
    setSelectedPreset('custom');
    setPresetNote(null);
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
            aria-label={`Check level for ${row.original.schemaName}.${row.original.tableName}`}
            value={row.original.level}
            onChange={(e) => updateLevel(row.original.tableId, e.target.value as CheckLevel)}
            className="w-48"
          />
        ),
      }),
      col.accessor('controlTotalColumns', {
        header: () => (
          <Tooltip
            label="Control totals"
            explanation="Number columns, such as amounts and balances, that Dmux adds up on both sides. The sums must match, so you know the money adds up the same."
          >
            Control totals
          </Tooltip>
        ),
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
      <ScreenHeader title="Choose checks" subtitle="Pick how thoroughly each table is checked. A full-values check compares every value." />
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
                options={PRESETS}
                value={selectedPreset}
                onChange={(e) => choosePreset(e.target.value as PresetId)}
                className="w-96"
                data-testid="preset-select"
              />
              <div className="rounded-lg border border-slate-200 px-3 py-2 text-sm dark:border-slate-700">
                <span className="font-medium">Estimated time:</span>{' '}
                {formatDurationRange(plan.estimateSecondsMin, plan.estimateSecondsMax)}
                <span className="block text-xs text-slate-500 dark:text-slate-400">
                  Corrected as each table finishes. A table that fails takes a second pass to list
                  its differences.
                </span>
              </div>
            </div>
            {presetNote && (
              <p role="status" className="text-sm text-slate-700 dark:text-slate-300">
                {presetNote}
              </p>
            )}
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
          <div className="flex items-center justify-between">
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
