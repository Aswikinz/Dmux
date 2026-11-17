import { useEffect, useMemo, useState } from 'react';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { StructureCompareView } from '../components/StructureCompareView';
import { rowsFromNodes } from '../components/structureRows';
import { ErrorCard } from '../design/ErrorCard';
import { Switch } from '../components/Switch';
import { Info } from 'lucide-react';
import type { Engine } from '../api/engine';
import type { ConnectionProfile, Inventory } from '../api/contract';

interface CompareStructureScreenProps {
  engine: Engine;
  oldProfile: ConnectionProfile;
  newProfile: ConnectionProfile;
  inventory: Inventory | null;
  onInventoryChange: (inventory: Inventory) => void;
  onNext: () => void;
  onBack: () => void;
}

/** "Label · PostgreSQL 9.6 · LATIN1", leaving out what is not known. */
function sideDetail(label: string, version: string | undefined, encoding: string): string {
  return [label, version ? `PostgreSQL ${version}` : '', encoding].filter(Boolean).join(' · ');
}

export function CompareStructureScreen({
  engine,
  oldProfile,
  newProfile,
  inventory,
  onInventoryChange,
  onNext,
  onBack,
}: CompareStructureScreenProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showOnlyProblems, setShowOnlyProblems] = useState(false);
  // Older backends send only the tree of differences; the view reads either.
  const rows = useMemo(
    () => inventory?.rows ?? (inventory ? rowsFromNodes(inventory.schemas) : []),
    [inventory],
  );

  useEffect(() => {
    let mounted = true;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        const result = await engine.compareStructure(oldProfile, newProfile);
        if (mounted) {
          onInventoryChange(result);
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
    if (!inventory) {
      void load();
    }
    return () => {
      mounted = false;
    };
  }, [engine, oldProfile, newProfile, inventory, onInventoryChange]);

  return (
    <div className="space-y-6">
      <ScreenHeader
        title="Compare structure"
        subtitle="Every table, column, index and constraint, side by side. Red is missing in the new database, green is added, amber is changed."
        actions={
          <Switch
            label="Show only problems"
            checked={showOnlyProblems}
            onChange={setShowOnlyProblems}
          />
        }
      />
      {loading && (
        <Panel>
          <div className="py-8 text-center text-slate-600 dark:text-slate-300">Loading inventory…</div>
        </Panel>
      )}
      {error && (
        <ErrorCard title="Could not compare structure">
          {error}
        </ErrorCard>
      )}
      {inventory && (
        <>
          <Panel className="flex flex-col gap-4">
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <span className="font-medium">Old encoding:</span> {inventory.oldEncoding}
              </div>
              <div>
                <span className="font-medium">New encoding:</span> {inventory.newEncoding}
              </div>
              <div>
                <span className="font-medium">Old collation:</span> {inventory.oldCollation}
              </div>
              <div>
                <span className="font-medium">New collation:</span> {inventory.newCollation}
              </div>
            </div>
            {inventory.oldEncoding !== inventory.newEncoding && (
              <div
                role="note"
                data-testid="encoding-note"
                className="flex animate-fade-up items-start gap-2.5 rounded-xl border border-accent-200/70 bg-accent-50/70 p-3.5 text-sm text-slate-800 dark:border-accent-500/25 dark:bg-accent-500/10 dark:text-slate-200"
              >
                <Info className="mt-0.5 h-4 w-4 shrink-0 text-accent-600 dark:text-accent-300" aria-hidden="true" />
                <p>
                  <span className="font-medium">For information: the text encodings differ</span> (
                  {inventory.oldEncoding} in the old database, {inventory.newEncoding} in the new
                  one). This is expected when moving to a newer server. Dmux converts every value to
                  UTF-8 before comparing, so accented names still compare correctly. You do not need
                  to do anything.
                </p>
              </div>
            )}
            <StructureCompareView
              rows={rows}
              showOnlyProblems={showOnlyProblems}
              old={{
                title: 'Old database',
                detail: sideDetail(oldProfile.label, inventory.oldServerVersion, inventory.oldEncoding),
              }}
              new={{
                title: 'New database',
                detail: sideDetail(newProfile.label, inventory.newServerVersion, inventory.newEncoding),
              }}
            />
          </Panel>
          <div className="flex items-center justify-between">
            <Button onClick={onBack} variant="secondary">
              Back
            </Button>
            <Button onClick={onNext} variant="primary" size="lg">
              Map tables
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
