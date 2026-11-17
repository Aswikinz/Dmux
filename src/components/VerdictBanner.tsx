import { AlertCircle, AlertTriangle, CheckCircle, XCircle } from 'lucide-react';
import { cn } from '../design/cn';
import { VERDICT_WORDS } from '../design/StatusBadge';
import type { CheckLevel, Results, Verdict } from '../api/contract';

/** The single, consistent verdict label: a PASS is never bare when options
 *  loosened the compare or the file cannot vouch for them (H2 / decision 23). */
export function qualifiedVerdictLabel(
  results: Pick<Results, 'verdict' | 'optionsApplied' | 'optionsRecorded'>,
): string | undefined {
  if (results.verdict !== 'PASS') {
    return undefined;
  }
  if (!results.optionsRecorded) {
    return 'PASS (options not recorded — older results file)';
  }
  if (results.optionsApplied) {
    return 'PASS (with options)';
  }
  return undefined;
}

/** The same level names as the Choose checks screen. */
export const LEVEL_LABELS: Record<CheckLevel, string> = {
  structure: 'Structure only',
  counts: 'Counts and totals',
  'full-values': 'Full values',
};

/** What each overall verdict means, in plain words. */
export const VERDICT_MEANING: Record<Verdict, string> = {
  PASS: 'The two databases match at the levels you chose.',
  FAIL: 'The two databases do not match. The failures below say what differs.',
  WIDELY_DIFFERENT:
    'Many rows differ, so Dmux stopped listing them one by one. The failures below show a sample.',
  NEEDS_REVIEW:
    'Dmux could not fully confirm a match. A person must review the items below before sign-off.',
  CHANGED_DURING_CHECK:
    'Data changed while Dmux was reading it, so this result cannot be relied on. Run the check again on a database that is not being written to.',
  TOOL_FAULT:
    'Dmux could not complete a check. This says nothing about your data; the reasons are listed below.',
  WARNING:
    'No difference was found, but something needs your attention. The warnings are listed below.',
};

const tone = (card: string, disc: string) => ({ card, disc });
const RED = tone(
  'border-red-200 bg-gradient-to-b from-red-50 to-white text-red-950 dark:border-red-500/30 dark:from-red-500/15 dark:to-red-500/5 dark:text-red-50',
  'bg-red-600 dark:bg-red-500',
);
const AMBER = tone(
  'border-amber-200 bg-gradient-to-b from-amber-50 to-white text-amber-950 dark:border-amber-500/30 dark:from-amber-500/15 dark:to-amber-500/5 dark:text-amber-50',
  'bg-amber-500',
);
const STYLE: Record<Verdict, { icon: typeof CheckCircle; card: string; disc: string }> = {
  PASS: {
    icon: CheckCircle,
    ...tone(
      'border-emerald-200 bg-gradient-to-b from-emerald-50 to-white text-emerald-950 dark:border-emerald-500/30 dark:from-emerald-500/15 dark:to-emerald-500/5 dark:text-emerald-50',
      'bg-emerald-600 dark:bg-emerald-500',
    ),
  },
  FAIL: { icon: XCircle, ...RED },
  WIDELY_DIFFERENT: { icon: XCircle, ...RED },
  NEEDS_REVIEW: { icon: AlertTriangle, ...AMBER },
  CHANGED_DURING_CHECK: {
    icon: AlertCircle,
    ...tone(
      'border-orange-200 bg-gradient-to-b from-orange-50 to-white text-orange-950 dark:border-orange-500/30 dark:from-orange-500/15 dark:to-orange-500/5 dark:text-orange-50',
      'bg-orange-600 dark:bg-orange-500',
    ),
  },
  TOOL_FAULT: {
    icon: AlertCircle,
    ...tone(
      'border-violet-200 bg-gradient-to-b from-violet-50 to-white text-violet-950 dark:border-violet-500/30 dark:from-violet-500/15 dark:to-violet-500/5 dark:text-violet-50',
      'bg-violet-600 dark:bg-violet-500',
    ),
  },
  WARNING: { icon: AlertTriangle, ...AMBER },
};

interface VerdictBannerProps {
  verdict: Verdict;
  /** A qualified label such as "PASS (with options)"; defaults to the verdict word. */
  label?: string;
  className?: string;
}

/**
 * The overall verdict, stated prominently in words: an icon, the verdict word
 * and one plain sentence.  Colour is only a third signal, never the only one.
 * An unrecognised verdict (untrusted results) is shown as such, never as PASS.
 */
export function VerdictBanner({ verdict, label, className }: VerdictBannerProps) {
  const style = STYLE[verdict];
  if (!style) {
    return (
      <div
        role="status"
        className={cn(
          'rounded-2xl border border-slate-300 bg-white p-5 text-slate-900 dark:border-white/15 dark:bg-card-dark dark:text-slate-100',
          className,
        )}
      >
        <p className="text-2xl font-bold">Result: UNKNOWN</p>
        <p className="mt-1 text-sm">Dmux does not recognise this result. Do not sign off on it.</p>
      </div>
    );
  }
  const Icon = style.icon;
  return (
    <div
      role="status"
      data-testid="verdict-banner"
      className={cn(
        'flex animate-scale-in items-center gap-4 rounded-2xl border p-5 shadow-[0_8px_24px_-16px_rgba(0,0,0,0.25)]',
        style.card,
        className,
      )}
    >
      <span
        className={cn(
          'flex h-12 w-12 shrink-0 animate-pop items-center justify-center rounded-full text-white shadow-[0_4px_12px_-4px_rgba(0,0,0,0.4)]',
          style.disc,
        )}
      >
        <Icon className="h-7 w-7" strokeWidth={2.25} aria-hidden="true" />
      </span>
      <div>
        <p className="text-[26px] font-bold leading-tight tracking-[-0.02em]">
          <span className="mr-1 align-middle text-[11px] font-semibold uppercase tracking-[0.08em]">
            Result:{' '}
          </span>
          <span data-testid="verdict-word">{label ?? VERDICT_WORDS[verdict]}</span>
        </p>
        <p className="mt-0.5 text-sm">{VERDICT_MEANING[verdict]}</p>
      </div>
    </div>
  );
}
