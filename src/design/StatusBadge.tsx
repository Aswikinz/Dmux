import { CheckCircle, AlertCircle, AlertTriangle, XCircle, Info } from 'lucide-react';
import { cn } from './cn';
import type { Verdict } from '../api/contract';

interface StatusBadgeProps {
  status: Verdict | 'idle' | 'running' | 'paused' | 'cancelled' | 'queued' | 'done' | 'same' | 'missing' | 'added' | 'changed';
  className?: string;
}

const config: Record<
  StatusBadgeProps['status'],
  { icon: typeof CheckCircle; label: string; classes: string }
> = {
  PASS: { icon: CheckCircle, label: 'Pass', classes: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300' },
  FAIL: { icon: XCircle, label: 'Fail', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  WARNING: { icon: AlertTriangle, label: 'Warning', classes: 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300' },
  CHANGED_DURING_CHECK: { icon: AlertCircle, label: 'Changed during check', classes: 'bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300' },
  idle: { icon: Info, label: 'Idle', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  running: { icon: Info, label: 'Running', classes: 'bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300' },
  paused: { icon: Info, label: 'Paused', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  cancelled: { icon: AlertCircle, label: 'Cancelled', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  queued: { icon: Info, label: 'Queued', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  done: { icon: CheckCircle, label: 'Done', classes: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300' },
  same: { icon: CheckCircle, label: 'Same', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  missing: { icon: XCircle, label: 'Missing', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  added: { icon: CheckCircle, label: 'Added', classes: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300' },
  changed: { icon: AlertTriangle, label: 'Changed', classes: 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300' },
};

export function StatusBadge({ status, className }: StatusBadgeProps) {
  const { icon: Icon, label, classes } = config[status] ?? config.idle;
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-sm font-medium',
        classes,
        className,
      )}
    >
      <Icon className="h-4 w-4" aria-hidden="true" />
      {label}
    </span>
  );
}
