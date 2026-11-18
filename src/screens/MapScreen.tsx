import { useEffect, useState } from 'react';
import { describeError } from '../api/tauriEngine';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { MappingQueue } from '../components/MappingQueue';
import { ErrorCard } from '../design/ErrorCard';
import type { Engine } from '../api/engine';
import { RuleEditor } from '../components/RuleEditor';
import { MatchedList } from '../components/MatchedList';
import type { PickerOption } from '../components/PairPicker';
import type {
  Inventory,
  Mapping,
  MappingEdit,
  MappingMatch,
  MappingRule,
  MappingSuggestion,
} from '../api/contract';

/** The edit that pairs `item` with the new-side name `value`. */
function pairEdit(item: MappingMatch, value: string): MappingEdit | null {
  const ref = item.ref;
  if (!ref) return null;
  return ref.column === undefined
    ? { kind: 'pair_table', schema: ref.schema, table: ref.table, newTable: value }
    : {
        kind: 'pair_column',
        schema: ref.schema,
        table: ref.table,
        column: ref.column,
        newColumn: value,
      };
}

interface MapScreenProps {
  engine: Engine;
  inventory: Inventory;
  mapping: Mapping | null;
  onMappingChange: (mapping: Mapping) => void;
  onNext: () => void;
  onBack: () => void;
}

export function MapScreen({
  engine,
  inventory,
  mapping,
  onMappingChange,
  onNext,
  onBack,
}: MapScreenProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveNote, setSaveNote] = useState<{ text: string; saved: boolean } | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [showMatched, setShowMatched] = useState(false);
  const [editBusy, setEditBusy] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        const result = await engine.buildMapping(inventory);
        if (mounted) {
          onMappingChange(result);
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
    if (!mapping) {
      void load();
    }
    return () => {
      mounted = false;
    };
  }, [engine, inventory, mapping, onMappingChange]);

  const handleApprove = (id: string) => {
    if (!mapping) return;
    const next: Mapping = {
      ...mapping,
      matches: mapping.matches.map((m) => (m.id === id ? { ...m, approved: true } : m)),
      needsAttention: mapping.needsAttention.filter((m) => m.id !== id),
    };
    onMappingChange(next);
  };

  /** Send changes to the engine, which checks every name against the live
   *  databases.  A refusal is shown as it is and leaves the mapping alone. */
  async function applyEdits(edits: MappingEdit[]) {
    if (!mapping) return;
    setEditBusy(true);
    setEditError(null);
    try {
      onMappingChange(await engine.editMapping(mapping, edits));
    } catch (e) {
      setEditError(describeError(e));
    } finally {
      setEditBusy(false);
    }
  }

  const handleAccept = (item: MappingMatch, suggestion: MappingSuggestion) => {
    const edit = pairEdit(item, suggestion.candidate);
    if (edit) void applyEdits([edit]);
  };

  const handlePick = (item: MappingMatch, value: string) => {
    const edit = pairEdit(item, value);
    if (edit) void applyEdits([edit]);
  };

  /** What the picker offers for an item: the new database's unmatched items of
   *  the same kind, plus any candidates the engine named. */
  const pickerOptions = (item: MappingMatch): PickerOption[] => {
    const ref = item.ref;
    if (!mapping || !ref) return [];
    const names = new Set<string>();
    for (const u of mapping.unmatchedNew ?? []) {
      if (u.ref.schema !== ref.schema) continue;
      if (ref.column === undefined && u.ref.column === undefined) names.add(u.ref.table);
      if (ref.column !== undefined && u.ref.column !== undefined && u.ref.table === ref.table) {
        names.add(u.ref.column);
      }
    }
    for (const c of item.candidates ?? []) names.add(c);
    return [...names].sort((a, b) => a.localeCompare(b)).map((n) => ({ value: n, label: n }));
  };

  const handlePreviewRules = (rules: MappingRule[]) => engine.previewRules(rules);

  const handleApplyRules = async (rules: MappingRule[]) => {
    setEditError(null);
    onMappingChange(await engine.rebuildMapping(rules));
  };

  const handleIgnore = (id: string, reason: string) => {
    if (!mapping) return;
    const item = mapping.needsAttention.find((m) => m.id === id);
    if (!item) return;
    // An item the engine named is ignored in the engine's own mapping, so the
    // run leaves it out; anything else is only noted here.
    if (item.ref) {
      void applyEdits([{ kind: 'ignore', path: item.id, reason }]);
      return;
    }
    const next: Mapping = {
      ...mapping,
      matches: mapping.matches.filter((m) => m.id !== id),
      needsAttention: mapping.needsAttention.filter((m) => m.id !== id),
      ignored: [
        ...mapping.ignored,
        { id: item.id, name: `${item.oldName} → ${item.newName}`, reason },
      ],
    };
    onMappingChange(next);
  };

  async function handleSave() {
    if (!mapping) return;
    setSaving(true);
    setSaveNote(null);
    setSaveError(null);
    try {
      const res = await engine.saveMapping(mapping);
      if (res.ok) {
        setSaveNote({
          text: `Mapping saved to ${res.filePath}${res.hash ? ` (mapping code ${res.hash.slice(0, 12)})` : ''}.`,
          saved: true,
        });
      } else if (res.cancelled) {
        setSaveNote({ text: 'Saving cancelled. Nothing was saved.', saved: false });
      } else if (res.demoOnly) {
        setSaveNote({
          text: 'This is the browser demo, so no file was saved. In the Dmux desktop app, you choose where to save the mapping.',
          saved: false,
        });
      } else {
        setSaveError(res.error ?? 'The mapping could not be saved.');
      }
    } catch (e) {
      setSaveError(describeError(e));
    } finally {
      setSaving(false);
    }
  }

  const tablePairs = mapping?.matches.filter((m) => m.kind !== 'column').length ?? 0;
  const columnPairs = mapping?.matches.filter((m) => m.kind === 'column').length ?? 0;

  return (
    <div className="space-y-6">
      <ScreenHeader title="Map tables and columns" subtitle="Confirm how tables and columns in the old database pair with the new one." />
      {loading && (
        <Panel>
          <div className="py-8 text-center text-slate-600 dark:text-slate-300">Building mapping…</div>
        </Panel>
      )}
      {error && <ErrorCard title="Could not build mapping">{error}</ErrorCard>}
      {mapping && (
        <>
          <Panel>
            <h2 className="mb-2 text-base font-semibold">Rename rules</h2>
            <RuleEditor
              rules={mapping.ruleSpecs ?? []}
              onPreview={handlePreviewRules}
              onApply={handleApplyRules}
            />
          </Panel>
          <Panel>
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
              <div className="text-sm text-slate-600 dark:text-slate-300" data-testid="matched-count">
                {tablePairs === 1 ? '1 table' : `${tablePairs} tables`} and{' '}
                {columnPairs === 1 ? '1 column' : `${columnPairs} columns`} paired
                {mapping.ignored.length > 0 && ` • ${mapping.ignored.length} ignored`}
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  onClick={() => setShowMatched((v) => !v)}
                  variant="secondary"
                  size="sm"
                  aria-expanded={showMatched}
                  aria-controls="matched-panel"
                >
                  {showMatched ? 'Hide what was mapped' : 'Show what was mapped'}
                </Button>
                <Button onClick={handleSave} disabled={saving} variant="secondary" size="sm">
                  {saving ? 'Saving…' : 'Save mapping'}
                </Button>
              </div>
            </div>
            {saveNote && (
              <p
                role="status"
                className={
                  saveNote.saved
                    ? 'mb-4 text-sm text-emerald-700 dark:text-emerald-300'
                    : 'mb-4 text-sm text-slate-700 dark:text-slate-300'
                }
              >
                {saveNote.text}
              </p>
            )}
            {saveError && (
              <ErrorCard title="The mapping was not saved" className="mb-4">
                {saveError}
              </ErrorCard>
            )}
            {showMatched && (
              <div
                id="matched-panel"
                className="mb-5 rounded-xl border border-slate-200 p-4 dark:border-slate-700"
              >
                <h3 className="mb-2 text-sm font-semibold">What was mapped</h3>
                <MatchedList matches={mapping.matches} />
              </div>
            )}
            {editError && (
              <ErrorCard title="That change was not made" className="mb-4">
                {editError}
              </ErrorCard>
            )}
            <MappingQueue
              queue={mapping.needsAttention}
              onApprove={handleApprove}
              onIgnore={handleIgnore}
              suggestions={mapping.suggestions}
              onAcceptSuggestion={handleAccept}
              pickerOptions={pickerOptions}
              onPick={handlePick}
              busy={editBusy}
            />
            {(mapping.unmatchedNew?.length ?? 0) > 0 && (
              <p className="mt-4 text-sm text-slate-600 dark:text-slate-300">
                {mapping.unmatchedNew?.length === 1
                  ? '1 item exists only in the new database.'
                  : `${mapping.unmatchedNew?.length} items exist only in the new database.`}{' '}
                They are offered when you choose a counterpart.
              </p>
            )}
            {mapping.ignored.length > 0 && (
              <div className="mt-4 rounded-lg border border-slate-200 p-3 dark:border-slate-700">
                <h3 className="mb-2 text-sm font-semibold">Ignored</h3>
                <ul className="space-y-1 text-sm">
                  {mapping.ignored.map((item) => (
                    <li key={item.id} className="text-slate-700 dark:text-slate-300">
                      <span className="font-medium">{item.name}</span>
                      <span className="ml-2 text-slate-500 dark:text-slate-400">— {item.reason}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </Panel>
          <div className="flex items-center justify-between">
            <Button onClick={onBack} variant="secondary">
              Back
            </Button>
            <Button onClick={onNext} variant="primary" size="lg">
              Choose checks
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
