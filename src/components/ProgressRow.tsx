import { cn } from '../design/cn';
import { StatusBadge } from '../design/StatusBadge';
import type { TableProgress } from '../api/contract';

interface ProgressRowProps {
  progress: TableProgress;
  className?: string;
}

export function ProgressRow({ progress, className }: ProgressRowProps) {
  const state =
    progress.state === 'done'
      ? 'PASS'
      : progress.state === 'failed'
        ? 'FAIL'
        : progress.state === 'changed-during-check'
          ? 'CHANGED_DURING_CHECK'
          : progress.state;

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
      <StatusBadge status={state} />
    </div>
  );
}
