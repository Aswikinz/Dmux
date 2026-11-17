import { useEffect, useState } from 'react';
import { Panel } from '../design/Panel';
import { Button } from '../design/Button';
import { MappingQueue } from '../components/MappingQueue';
import { ErrorCard } from '../design/ErrorCard';
import type { Engine } from '../api/engine';
import type { Inventory, Mapping } from '../api/contract';

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
          setError(e instanceof Error ? e.message : String(e));
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

  const handleIgnore = (id: string, reason: string) => {
    if (!mapping) return;
    const item = mapping.needsAttention.find((m) => m.id === id);
    if (!item) return;
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
    try {
      await engine.saveMapping(mapping);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">Map tables and columns</h2>
      {loading && (
        <Panel>
          <div className="py-8 text-center text-slate-600 dark:text-slate-300">Building mapping…</div>
        </Panel>
      )}
      {error && <ErrorCard title="Could not build mapping">{error}</ErrorCard>}
      {mapping && (
        <>
          <Panel>
            <div className="mb-4 flex items-center justify-between">
              <div className="text-sm text-slate-600 dark:text-slate-300">
                {mapping.matches.length} matched{' '}
                {mapping.ignored.length > 0 && `• ${mapping.ignored.length} ignored`}
              </div>
              <Button onClick={handleSave} disabled={saving} variant="secondary" size="sm">
                {saving ? 'Saving…' : 'Save mapping'}
              </Button>
            </div>
            <MappingQueue
              queue={mapping.needsAttention}
              onApprove={handleApprove}
              onIgnore={handleIgnore}
            />
            {mapping.ignored.length > 0 && (
              <div className="mt-4 rounded-lg border border-slate-200 p-3 dark:border-slate-700">
                <h3 className="mb-2 text-sm font-semibold">Ignored</h3>
                <ul className="space-y-1 text-sm">
                  {mapping.ignored.map((item) => (
                    <li key={item.id} className="text-slate-700 dark:text-slate-300">
                      <span className="font-medium">{item.name}</span>
                      <span className="ml-2 text-slate-500">— {item.reason}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </Panel>
          <div className="flex justify-between">
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
