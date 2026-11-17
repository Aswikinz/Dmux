import { cn } from '../design/cn';
import { StatusBadge } from '../design/StatusBadge';
import type { TableProgress } from '../api/contract';

interface ProgressRowProps {
  progress: TableProgress;
  className?: string;
}

/**
 * Shows run progress per table. It renders only the run *state* (done, running,
 * failed, changed-during-check, …). It must never fabricate a verdict: a scan
 * that finished is "Done", not "Pass", and a failed scan is "Failed", not a
 * data "Fail". The per-table verdict appears only on the results screen once
 * the engine has produced one.
 */
export function ProgressRow({ progress, className }: ProgressRowProps) {
  return (
    <div
      className={cn(
        'flex items-center gap-4 rounded-lg border border-slate-200 px-3 py-2 dark:border-slate-700',
        className,
      )}
    >
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-slate-900 dark:text-slate-100">
          {progress.schemaName}.{progress.tableName}
        </div>
        {progress.etaSeconds !== undefined && progress.state === 'running' && (
          <div className="text-xs text-slate-500 dark:text-slate-400">
            About {progress.etaSeconds}s left
          </div>
        )}
      </div>
      <div className="w-32">
        <div
          className="h-2 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700"
          aria-hidden="true"
        >
          <div
            className="h-full rounded-full bg-blue-600 transition-all duration-300 dark:bg-blue-500"
            style={{ width: `${progress.progressPercent}%` }}
          />
        </div>
      </div>
      <StatusBadge status={progress.state} />
    </div>
  );
}