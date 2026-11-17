import { useEffect, useState, useRef } from 'react';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { ProgressRow } from '../components/ProgressRow';
import { StatusBadge } from '../design/StatusBadge';
import { ErrorCard } from '../design/ErrorCard';
import { VerdictBanner, qualifiedVerdictLabel } from '../components/VerdictBanner';
import { formatDurationRange } from '../components/duration';
import { HookWarning } from '../components/HookWarning';
import type { Engine } from '../api/engine';
import type { RunDefinition, RunProgress, Verdict } from '../api/contract';

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
  // The overall verdict, fetched as soon as the run ends, so the screen never
  // shows a neutral "finished" next to a FAIL the user has not seen yet.
  const [verdict, setVerdict] = useState<{ word: Verdict; label?: string } | null>(null);
  const [verdictError, setVerdictError] = useState<string | null>(null);
  // Custom checks on: the user confirms the warning before the run starts.
  const [gate, setGate] = useState<'loading' | 'ask' | 'go'>(progress ? 'go' : 'loading');
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
    if (!progress && gate === 'go') {
      void start();
    }
    return () => {
      mounted = false;
    };
  }, [engine, definition, progress, onProgressChange, gate]);

  useEffect(() => {
    if (progress || gate !== 'loading') return;
    let mounted = true;
    engine
      .getSettings()
      .then((s) => {
        if (mounted) setGate(s.customChecksEnabled ? 'ask' : 'go');
      })
      .catch(() => {
        // If the setting cannot be read, the backend still decides; the run
        // result shows whether custom checks ran.
        if (mounted) setGate('go');
      });
    return () => {
      mounted = false;
    };
  }, [engine, progress, gate]);

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

  const runId = progress?.runId;
  const isDone = progress?.overallState === 'done';
  useEffect(() => {
    if (!isDone || !runId) return;
    let mounted = true;
    setVerdictError(null);
    engine
      .getResults(runId)
      .then((r) => {
        if (!mounted) return;
        setVerdict({ word: r.verdict, label: qualifiedVerdictLabel(r) });
      })
      .catch((e: unknown) => {
        if (mounted) setVerdictError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      mounted = false;
    };
  }, [engine, runId, isDone]);

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
      <ScreenHeader title="Run checks" subtitle="Dmux reads both databases and compares them. You can pause or cancel at any time." />
      {error && <ErrorCard title="Run error">{error}</ErrorCard>}
      {!progress && gate === 'ask' && (
        <Panel>
          <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">
            Custom checks are on
          </h3>
          <HookWarning className="mt-3">
            After the comparison, the scripts in your custom checks folder will run. You can turn them off in Settings.
          </HookWarning>
          <div className="mt-4 flex gap-3">
            <Button variant="secondary" onClick={onBack}>
              Back
            </Button>
            <Button variant="primary" onClick={() => setGate('go')}>
              Start with custom checks
            </Button>
          </div>
        </Panel>
      )}
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
                        ? 'Cancelled: this run has no verdict.'
                        : 'The run has finished.'}
                </div>
                {progress.overallState === 'running' && progress.waitingForWindow && (
                  <div role="status" className="text-sm font-medium text-slate-800 dark:text-slate-100">
                    Waiting for the run window (opens at {progress.waitingForWindow})
                  </div>
                )}
                {progress.notices?.map((n) => (
                  <div key={n} role="status" className="text-sm text-amber-800 dark:text-amber-300">
                    {n}
                  </div>
                ))}
                {!finished &&
                  progress.etaSecondsMin !== undefined &&
                  progress.etaSecondsMax !== undefined && (
                    <div className="text-sm text-slate-600 dark:text-slate-300">
                      Time remaining:{' '}
                      {formatDurationRange(progress.etaSecondsMin, progress.etaSecondsMax)}
                    </div>
                  )}
              </div>
              {progress.overallState !== 'done' && <StatusBadge status={progress.overallState} />}
            </div>
            {progress.overallState === 'done' && (
              <div className="mb-4">
                {verdict ? (
                  <VerdictBanner verdict={verdict.word} label={verdict.label} />
                ) : verdictError ? (
                  <ErrorCard title="Could not load the verdict">
                    {verdictError} Open the results to see what happened.
                  </ErrorCard>
                ) : (
                  <p className="text-sm text-slate-600 dark:text-slate-300">Getting the verdict…</p>
                )}
              </div>
            )}
            <div
              className="mb-4 max-h-[50vh] space-y-0.5 overflow-auto rounded-xl"
              tabIndex={0}
              role="region"
              aria-label="Progress per table"
            >
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
          <div className="flex items-center justify-between">
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
