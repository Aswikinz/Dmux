import { CheckCircle, AlertCircle, AlertTriangle, XCircle, Info, Flag } from 'lucide-react';
import { cn } from './cn';
import type { Verdict } from '../api/contract';

/** The one spelling of every verdict, used on every screen (never colour alone). */
export const VERDICT_WORDS: Record<Verdict, string> = {
  PASS: 'PASS',
  FAIL: 'FAIL',
  WARNING: 'WARNING',
  NEEDS_REVIEW: 'NEEDS REVIEW',
  CHANGED_DURING_CHECK: 'CHANGED DURING CHECK',
  TOOL_FAULT: 'TOOL FAULT',
  WIDELY_DIFFERENT: 'WIDELY DIFFERENT',
};

interface StatusBadgeProps {
  status:
    | Verdict
    | 'idle'
    | 'running'
    | 'paused'
    | 'cancelled'
    | 'queued'
    | 'done'
    | 'failed'
    | 'changed-during-check'
    | 'not-scanned'
    | 'same'
    | 'missing'
    | 'added'
    | 'changed'
    | 'missing-in-new'
    | 'extra-in-new'
    | 'changed-values';
  /** Optional override so the badge and any qualifier ("with options",
   *  "options not recorded") are a single, consistent label. */
  label?: string;
  className?: string;
}

const config: Record<
  StatusBadgeProps['status'] | 'unknown',
  { icon: typeof CheckCircle; label: string; classes: string }
> = {
  PASS: { icon: CheckCircle, label: 'PASS', classes: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300' },
  FAIL: { icon: XCircle, label: 'FAIL', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  WIDELY_DIFFERENT: { icon: AlertTriangle, label: 'WIDELY DIFFERENT', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  NEEDS_REVIEW: { icon: AlertTriangle, label: 'NEEDS REVIEW', classes: 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300' },
  CHANGED_DURING_CHECK: { icon: AlertCircle, label: 'CHANGED DURING CHECK', classes: 'bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300' },
  TOOL_FAULT: { icon: AlertCircle, label: 'TOOL FAULT', classes: 'bg-violet-100 text-violet-800 dark:bg-violet-900/40 dark:text-violet-300' },
  WARNING: { icon: AlertTriangle, label: 'WARNING', classes: 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300' },
  idle: { icon: Info, label: 'Idle', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  running: { icon: Info, label: 'Running', classes: 'bg-accent-50 text-accent-700 dark:bg-accent-500/15 dark:text-accent-300' },
  paused: { icon: Info, label: 'Paused', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  cancelled: { icon: AlertCircle, label: 'Cancelled', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  queued: { icon: Info, label: 'Queued', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  'not-scanned': { icon: Info, label: 'Not scanned', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  // A finished scan is not a verdict: a neutral flag, never a success tick.
  done: { icon: Flag, label: 'Scan finished', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  failed: { icon: XCircle, label: 'Scan failed', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  'changed-during-check': { icon: AlertCircle, label: 'CHANGED DURING CHECK', classes: 'bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300' },
  same: { icon: CheckCircle, label: 'Same', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
  missing: { icon: XCircle, label: 'Missing', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  added: { icon: CheckCircle, label: 'Added', classes: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300' },
  changed: { icon: AlertTriangle, label: 'Changed', classes: 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300' },
  'missing-in-new': { icon: XCircle, label: 'Missing in new', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  'extra-in-new': { icon: XCircle, label: 'Extra in new', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  'changed-values': { icon: XCircle, label: 'Changed values', classes: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300' },
  unknown: { icon: Info, label: 'Unknown', classes: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300' },
};

export function StatusBadge({ status, label, className }: StatusBadgeProps) {
  // An unrecognised verdict falls back to a neutral "Unknown", never PASS and
  // never "Idle". The engine's results JSON is untrusted input.
  const configEntry = config[status] ?? config.unknown;
  const { icon: Icon, classes } = configEntry;
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full px-2 py-0.5 text-xs font-semibold tracking-[0.01em] ring-1 ring-inset ring-current/15',
        classes,
        className,
      )}
    >
      <Icon className="h-3.5 w-3.5" strokeWidth={2.5} aria-hidden="true" />
      {label ?? configEntry.label}
    </span>
  );
}
