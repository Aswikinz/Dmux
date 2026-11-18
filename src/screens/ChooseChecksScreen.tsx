import { useEffect, useMemo, useRef, useState } from 'react';
import { describeError } from '../api/tauriEngine';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { Select } from '../design/Select';
import { Input } from '../design/Input';
import { DataTable } from '../components/DataTable';
import { ErrorCard } from '../design/ErrorCard';
import { Tooltip } from '../design/Tooltip';
import { formatDurationRange } from '../components/duration';
import { createColumnHelper, type ColumnDef } from '@tanstack/react-table';
import {
  MEGABYTE,
  formatSize,
  selectByPattern,
  selectBySchema,
  selectBySize,
  unknownSizeCount,
} from './checkSelection';
import type { Engine } from '../api/engine';
import type { CheckLevel, CheckPlan, Mapping, TableCheck } from '../api/contract';
import { RunWindowControl } from '../components/RunWindowControl';
import { TableScopePicker, isChosen } from '../components/TableScopePicker';

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

/** A fresh plan starts with no table chosen: the user picks what this run reads. */
export function withNoneChosen(plan: CheckPlan): CheckPlan {
  return { ...plan, tables: plan.tables.map((t) => ({ ...t, included: false })) };
}

/** The plan's estimate scaled to the chosen tables: by size when every table
 *  has one, else by count.  Whole seconds, at least 1. */
export function chosenEstimate(plan: CheckPlan): { min: number; max: number } {
  const chosen = plan.tables.filter(isChosen);
  if (chosen.length === plan.tables.length) {
    return { min: plan.estimateSecondsMin, max: plan.estimateSecondsMax };
  }
  const sized = plan.tables.every((t) => t.sizeBytes !== undefined);
  const total = sized ? plan.tables.reduce((n, t) => n + (t.sizeBytes ?? 0), 0) : plan.tables.length;
  const part = sized ? chosen.reduce((n, t) => n + (t.sizeBytes ?? 0), 0) : chosen.length;
  const share = total > 0 ? part / total : 0;
  return {
    min: Math.max(1, Math.round(plan.estimateSecondsMin * share)),
    max: Math.max(1, Math.round(plan.estimateSecondsMax * share)),
  };
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
  // The tables ticked for a bulk change, by table id.
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [selectionNote, setSelectionNote] = useState<string | null>(null);
  const [schemaChoice, setSchemaChoice] = useState('');
  const [patternText, setPatternText] = useState('');
  const [sizeDirection, setSizeDirection] = useState<'larger' | 'smaller'>('larger');
  const [sizeMegabytes, setSizeMegabytes] = useState('');
  const [bulkLevel, setBulkLevel] = useState<CheckLevel>('full-values');
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
        // The starting level from Settings ('suggested' keeps the engine's own).
        const start = await engine
          .getSettings()
          .then((st) => st.startingLevel)
          .catch(() => 'suggested' as const);
        const preset: PresetId =
          start === 'full-values' ? 'full' : start === 'suggested' ? 'suggested' : start;
        if (mounted) {
          suggestedRef.current = Object.fromEntries(result.tables.map((t) => [t.tableId, t.level]));
          onPlanChange(
            withNoneChosen(
              preset === 'suggested' ? result : applyPreset(result, preset, suggestedRef.current),
            ),
          );
          setSelectedPreset(preset);
        }
      } catch (e) {
        if (mounted) {
          setError(describeError(e));
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

  const replaceSelection = (ids: string[], what: string) => {
    setSelected(new Set(ids));
    setSelectionNote(
      ids.length === 1 ? `1 table selected (${what}).` : `${ids.length} tables selected (${what}).`,
    );
  };

  const toggleSelected = (tableId: string, on: boolean) => {
    setSelectionNote(null);
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(tableId);
      else next.delete(tableId);
      return next;
    });
  };

  const applyBulkLevel = () => {
    const ids = new Set([...selected].filter((id) => plan?.tables.some((t) => t.tableId === id && isChosen(t))));
    if (!plan || ids.size === 0) return;
    setSelectedPreset('custom');
    setPresetNote(null);
    onPlanChange({
      ...plan,
      tables: plan.tables.map((t) => (ids.has(t.tableId) ? { ...t, level: bulkLevel } : t)),
    });
    const label = levelOptions.find((o) => o.value === bulkLevel)?.label ?? bulkLevel;
    setSelectionNote(`Set ${ids.size === 1 ? '1 table' : `${ids.size} tables`} to "${label}".`);
  };

  /** Tick or untick a suggested control-total column.  Only ticked columns are
   *  checked as control totals. */
  const toggleControl = (tableId: string, column: string, on: boolean) => {
    if (!plan) return;
    setPresetNote(null);
    onPlanChange({
      ...plan,
      tables: plan.tables.map((t) => {
        if (t.tableId !== tableId) return t;
        const confirmed = new Set(t.controlTotalColumns ?? []);
        if (on) confirmed.add(column);
        else confirmed.delete(column);
        return { ...t, controlTotalColumns: confirmed.size > 0 ? [...confirmed] : undefined };
      }),
    });
  };

  // The table's cells read the latest state and handlers through this ref, so
  // the column list keeps its identity.  A new column list makes the table
  // rebuild every cell, which would drop keyboard focus after each tick.
  const live = useRef({ selected, toggleSelected, toggleControl, updateLevel });
  live.current = { selected, toggleSelected, toggleControl, updateLevel };

  const columns = useMemo(() => {
    const col = createColumnHelper<TableCheck>();
    return [
      col.display({
        id: 'select',
        header: () => <span className="sr-only">Select</span>,
        size: 60,
        cell: ({ row }) => (
          <input
            type="checkbox"
            className="h-4 w-4 accent-accent-600"
            aria-label={`Select ${row.original.schemaName}.${row.original.tableName}`}
            checked={live.current.selected.has(row.original.tableId)}
            onChange={(e) => live.current.toggleSelected(row.original.tableId, e.target.checked)}
          />
        ),
      }),
      col.accessor('schemaName', { header: 'Schema' }),
      col.accessor('tableName', { header: 'Table' }),
      col.accessor('level', {
        header: 'Check level',
        cell: ({ row }) => (
          <Select
            options={levelOptions}
            aria-label={`Check level for ${row.original.schemaName}.${row.original.tableName}`}
            value={row.original.level}
            onChange={(e) =>
              live.current.updateLevel(row.original.tableId, e.target.value as CheckLevel)
            }
            className="w-48"
          />
        ),
      }),
      col.accessor('sizeBytes', {
        header: 'Size',
        size: 90,
        cell: ({ getValue }) => formatSize(getValue() as number | undefined),
      }),
      col.display({
        id: 'controlTotals',
        header: () => (
          <Tooltip
            label="Control totals"
            explanation="Number columns, such as amounts and balances, that Dmux adds up on both sides. The sums must match, so you know the money adds up the same."
          >
            Control totals
          </Tooltip>
        ),
        size: 220,
        cell: ({ row }) => {
          const t = row.original;
          const suggestions = t.controlTotalSuggestions ?? [];
          if (suggestions.length === 0) return '—';
          return (
            <div className="flex flex-nowrap gap-x-3 whitespace-nowrap">
              {suggestions.map((column) => (
                <label key={column} className="flex items-center gap-1.5 text-sm">
                  <input
                    type="checkbox"
                    className="h-4 w-4 accent-accent-600"
                    aria-label={`Use ${column} as a control total for ${t.schemaName}.${t.tableName}`}
                    checked={(t.controlTotalColumns ?? []).includes(column)}
                    onChange={(e) => live.current.toggleControl(t.tableId, column, e.target.checked)}
                  />
                  {column}
                </label>
              ))}
            </div>
          );
        },
      }),
    ] as ColumnDef<TableCheck>[];
  }, []);

  // Only the chosen tables take a level; the rest are not read.
  const chosen = useMemo(() => (plan?.tables ?? []).filter(isChosen), [plan]);

  const schemas = useMemo(
    () => [...new Set(chosen.map((t) => t.schemaName))].sort((a, b) => a.localeCompare(b)),
    [chosen],
  );

  const totals = useMemo(() => {
    const full = chosen.filter((t) => t.level === 'full-values').length;
    const counts = chosen.filter((t) => t.level === 'counts').length;
    const structure = chosen.filter((t) => t.level === 'structure').length;
    return { full, counts, structure };
  }, [chosen]);

  // A bulk selection only ever counts tables that are still chosen.
  const chosenIds = useMemo(() => new Set(chosen.map((t) => t.tableId)), [chosen]);
  const selectedChosen = useMemo(
    () => new Set([...selected].filter((id) => chosenIds.has(id))),
    [selected, chosenIds],
  );
  const estimate = plan ? chosenEstimate(plan) : null;

  return (
    <div className="space-y-6">
      <ScreenHeader title="Choose checks" subtitle="Choose which tables this run checks, then how thoroughly. A full-values check compares every value." />
      {loading && (
        <Panel>
          <div className="py-8 text-center text-slate-600 dark:text-slate-300">Building check plan…</div>
        </Panel>
      )}
      {error && <ErrorCard title="Could not build check plan">{error}</ErrorCard>}
      {plan && (
        <>
          <Panel>
            <TableScopePicker tables={plan.tables} onChange={(tables) => onPlanChange({ ...plan, tables })} />
          </Panel>
          {chosen.length === 0 ? (
            <Panel>
              <p className="text-sm text-slate-600 dark:text-slate-300" data-testid="levels-waiting">
                Choose at least one table above. Then you can set how thoroughly each one is checked.
              </p>
            </Panel>
          ) : (
            <Panel className="flex flex-col gap-4">
              <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">
                How thoroughly to check the chosen tables
              </h3>
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
                  {formatDurationRange(estimate?.min ?? plan.estimateSecondsMin, estimate?.max ?? plan.estimateSecondsMax)}
                  {chosen.length < plan.tables.length && (
                    <span className="text-slate-500 dark:text-slate-400">
                      {' '}
                      for the {chosen.length === 1 ? '1 chosen table' : `${chosen.length} chosen tables`}
                    </span>
                  )}
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
              {chosen.length > 0 && (
                <div className="flex gap-4 text-sm text-slate-600 dark:text-slate-300">
                  <span><strong>{totals.full}</strong> full values</span>
                  <span><strong>{totals.counts}</strong> counts</span>
                  <span><strong>{totals.structure}</strong> structure only</span>
                </div>
              )}
              <div
                role="group"
                aria-label="Select tables"
                className="space-y-3 rounded-lg border border-slate-200 p-3 dark:border-slate-700"
              >
                <p className="text-sm font-medium">Select several chosen tables, then change their level together</p>
                <div className="flex flex-wrap items-end gap-x-8 gap-y-4">
                  <div className="flex items-end gap-2" data-testid="select-by-schema">
                    <Select
                      label="Schema"
                      options={[{ value: '', label: 'Choose a schema' }, ...schemas.map((x) => ({ value: x, label: x }))]}
                      value={schemaChoice}
                      onChange={(e) => setSchemaChoice(e.target.value)}
                      className="w-44"
                    />
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={!schemaChoice}
                      onClick={() =>
                        replaceSelection(selectBySchema(chosen, schemaChoice), `schema ${schemaChoice}`)
                      }
                    >
                      Select schema
                    </Button>
                  </div>
                  <div className="flex items-end gap-2" data-testid="select-by-name">
                    <Input
                      label="Name pattern"
                      placeholder="acct_* (* matches anything)"
                      value={patternText}
                      onChange={(e) => setPatternText(e.target.value)}
                      className="w-52"
                    />
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={!patternText.trim()}
                      onClick={() =>
                        replaceSelection(selectByPattern(chosen, patternText), `names matching ${patternText.trim()}`)
                      }
                    >
                      Select by name
                    </Button>
                  </div>
                  <div className="flex items-end gap-2" data-testid="select-by-size">
                    <Select
                      label="Size"
                      options={[
                        { value: 'larger', label: 'Larger than' },
                        { value: 'smaller', label: 'Smaller than' },
                      ]}
                      value={sizeDirection}
                      onChange={(e) => setSizeDirection(e.target.value as 'larger' | 'smaller')}
                      className="w-36"
                    />
                    <Input
                      label="Megabytes"
                      type="number"
                      min={0}
                      value={sizeMegabytes}
                      onChange={(e) => setSizeMegabytes(e.target.value)}
                      className="w-24"
                    />
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={sizeMegabytes.trim() === '' || !(Number(sizeMegabytes) >= 0)}
                      onClick={() =>
                        replaceSelection(
                          selectBySize(chosen, sizeDirection, Number(sizeMegabytes)),
                          `${sizeDirection} than ${Number(sizeMegabytes) * MEGABYTE === 0 ? '0' : sizeMegabytes} MB`,
                        )
                      }
                    >
                      Select by size
                    </Button>
                  </div>
                </div>
                <div className="flex flex-wrap items-end gap-3">
                  <Select
                    label="Set level for the selection"
                    options={levelOptions}
                    value={bulkLevel}
                    onChange={(e) => setBulkLevel(e.target.value as CheckLevel)}
                    className="w-52"
                  />
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={selectedChosen.size === 0}
                    onClick={applyBulkLevel}
                  >
                    Apply to {selectedChosen.size} selected
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={selectedChosen.size === 0}
                    onClick={() => {
                      setSelected(new Set());
                      setSelectionNote('Selection cleared.');
                    }}
                  >
                    Clear selection
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() =>
                      replaceSelection(
                        chosen.map((t) => t.tableId),
                        'all chosen tables',
                      )
                    }
                  >
                    Select all listed
                  </Button>
                </div>
                {selectionNote && (
                  <p role="status" className="text-sm text-slate-700 dark:text-slate-300">
                    {selectionNote}
                  </p>
                )}
                {unknownSizeCount(chosen) > 0 && (
                  <p className="text-xs text-slate-600 dark:text-slate-400">
                    {unknownSizeCount(chosen)} tables have no size on record, so selecting by size skips them.
                  </p>
                )}
              </div>
              <p className="text-xs text-slate-600 dark:text-slate-400">
                A control-total column you tick is added up in every table that has a column with that
                name. Nothing is added up until you tick it.
              </p>
              <DataTable
                data={chosen}
                columns={columns}
                filterPlaceholder="Filter by schema or table name"
              />
            </Panel>
          )}
          <RunWindowControl
            value={plan.runWindow}
            onChange={(runWindow) => onPlanChange({ ...plan, runWindow })}
          />
          <div className="flex items-center justify-between">
            <Button onClick={onBack} variant="secondary">
              Back
            </Button>
            <div className="flex items-center gap-4">
              {chosen.length === 0 && (
                <span className="text-sm text-slate-600 dark:text-slate-300">
                  Choose at least one table to run.
                </span>
              )}
              <Button onClick={onNext} variant="primary" size="lg" disabled={chosen.length === 0}>
                Run
              </Button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
