import { useEffect, useState } from 'react';
import { Panel } from '../design/Panel';
import { Button } from '../design/Button';
import { DiffTree } from '../components/DiffTree';
import { ErrorCard } from '../design/ErrorCard';
import { Switch } from '../components/Switch';
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
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">
          Compare structure
        </h2>
        <Switch
          label="Show only problems"
          checked={showOnlyProblems}
          onChange={setShowOnlyProblems}
        />
      </div>
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
            <DiffTree nodes={inventory.schemas} showOnlyProblems={showOnlyProblems} />
          </Panel>
          <div className="flex justify-between">
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
