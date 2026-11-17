import { useEffect, useState, useRef } from 'react';
import { Panel } from '../design/Panel';
import { Button } from '../design/Button';
import { ProgressRow } from '../components/ProgressRow';
import { StatusBadge } from '../design/StatusBadge';
import { ErrorCard } from '../design/ErrorCard';
import type { Engine } from '../api/engine';
import type { RunDefinition, RunProgress } from '../api/contract';

interface RunScreenProps {
  engine: Engine;
  definition: RunDefinition;
  progress: RunProgress | null;
  onProgressChange: (progress: RunProgress) => void;
  onNext: () => void;
  onBack: () => void;
}

export function RunScreen({
  engine,
  definition,
  progress,
  onProgressChange,
  onNext,
  onBack,
}: RunScreenProps) {
  const [error, setError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    let mounted = true;
    async function start() {
      try {
        const initial = await engine.startRun(definition);
        if (mounted) {
          onProgressChange(initial);
        }
      } catch (e) {
        if (mounted) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    }
    if (!progress) {
      void start();
    }
    return () => {
      mounted = false;
    };
  }, [engine, definition, progress, onProgressChange]);

  useEffect(() => {
    if (!progress || progress.overallState === 'done' || progress.overallState === 'cancelled') {
      return;
    }
    intervalRef.current = setInterval(async () => {
      try {
        const next = await engine.pollRun(progress.runId);
        onProgressChange(next);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    }, 500);
    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, [engine, progress, onProgressChange]);

  async function togglePause() {
    if (!progress) return;
    try {
      const next = await engine.setRunPaused(
        progress.runId,
        progress.overallState === 'running',
      );
      onProgressChange(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function cancel() {
    if (!progress) return;
    setCancelling(true);
    try {
      const next = await engine.cancelRun(progress.runId);
      onProgressChange(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCancelling(false);
    }
  }

  const finished =
    progress && (progress.overallState === 'done' || progress.overallState === 'cancelled');

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">Run checks</h2>
      {error && <ErrorCard title="Run error">{error}</ErrorCard>}
      {progress && (
        <>
          <Panel>
            <div className="mb-4 flex items-center justify-between">
              <div>
                <div className="text-sm text-slate-600 dark:text-slate-300">
                  {progress.overallState === 'running'
                    ? 'Running'
                    : progress.overallState === 'paused'
                      ? 'Paused'
                      : progress.overallState === 'cancelled'
                        ? 'Cancelled'
                        : 'Finished'}
                </div>
                {progress.etaSecondsMin !== undefined && progress.etaSecondsMax !== undefined && (
                  <div className="text-sm text-slate-600 dark:text-slate-300">
                    Time remaining: {progress.etaSecondsMin}s – {progress.etaSecondsMax}s
                  </div>
                )}
              </div>
              <StatusBadge status={progress.overallState} />
            </div>
            <div className="mb-4 max-h-[50vh] space-y-2 overflow-auto">
              {progress.tables.map((t) => (
                <ProgressRow key={t.tableId} progress={t} />
              ))}
            </div>
            <div className="flex gap-3">
              <Button
                onClick={togglePause}
                variant="secondary"
                disabled={finished || progress.overallState === 'cancelled'}
              >
                {progress.overallState === 'paused' ? 'Resume' : 'Pause'}
              </Button>
              <Button
                onClick={cancel}
                variant="danger"
                disabled={finished || cancelling}
              >
                {cancelling ? 'Cancelling…' : 'Cancel'}
              </Button>
            </div>
          </Panel>
          <div className="flex justify-between">
            <Button onClick={onBack} variant="secondary">
              Back
            </Button>
            <Button onClick={onNext} disabled={!finished} variant="primary" size="lg">
              See results
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
