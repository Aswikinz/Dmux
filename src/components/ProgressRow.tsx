import { cn } from '../design/cn';
import { StatusBadge } from '../design/StatusBadge';
import type { TableProgress } from '../api/contract';
import { formatDuration } from './duration';

interface ProgressRowProps {
  progress: TableProgress;
  className?: string;
}

/**
 * Shows run progress per table. It renders only the run *state* (done, running,
 * failed, changed-during-check, …). It must never fabricate a verdict: a scan
 * that finished is "Scan finished", never PASS, and a failed scan is "Scan
 * failed", not the data verdict FAIL. The per-table verdict appears only on the results screen once
 * the engine has produced one.
 */
export function ProgressRow({ progress, className }: ProgressRowProps) {
  return (
    <div
      className={cn(
        'flex items-center gap-4 rounded-xl px-3 py-2.5 transition-colors hover:bg-black/[0.025] dark:hover:bg-white/[0.04]',
        className,
      )}
    >
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-slate-900 dark:text-slate-100">
          {progress.schemaName}.{progress.tableName}
        </div>
        {progress.etaSeconds !== undefined && progress.state === 'running' && (
          <div className="text-xs text-slate-500 dark:text-slate-400">
            {formatDuration(progress.etaSeconds)} left
          </div>
        )}
      </div>
      <div className="w-32">
        <div
          className="h-1.5 overflow-hidden rounded-full bg-black/[0.07] dark:bg-white/10"
          aria-hidden="true"
        >
          <div
            className="h-full rounded-full bg-gradient-to-r from-accent-500 to-accent-600 transition-[width] duration-500 ease-(--ease-apple) dark:from-accent-400 dark:to-accent-500"
            style={{ width: `${progress.progressPercent}%` }}
          />
        </div>
      </div>
      <StatusBadge status={progress.state} />
    </div>
  );
}
