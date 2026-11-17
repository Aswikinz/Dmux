import { Panel } from '../design/Panel';
import { StatusBadge } from '../design/StatusBadge';
import type { CustomChecks, CustomSeverity } from '../api/contract';

const SEVERITY_LABEL: Record<CustomSeverity, string> = {
  fail: 'Failed',
  warning: 'Warning',
  info: 'Note',
};

/** The results of the user's own scripts.  Findings are shown as text only
 *  (the backend has already removed control characters). */
export function CustomChecksPanel({ checks }: { checks: CustomChecks }) {
  return (
    <Panel>
      <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">
        Custom checks
      </h3>
      <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">
        These findings come from scripts you chose. A failed custom check makes the verdict FAIL.
      </p>
      {checks.findings.length === 0 && checks.warnings.length === 0 && (
        <p className="mt-3 text-sm text-slate-700 dark:text-slate-300">
          The scripts ran and reported nothing.
        </p>
      )}
      {checks.findings.length > 0 && (
        <ul className="mt-3 space-y-2" aria-label="Custom check findings">
          {checks.findings.map((f, i) => (
            <li
              key={`${f.id}-${i}`}
              className="rounded-xl border border-black/[0.08] p-3 text-sm dark:border-white/10"
            >
              <div className="flex flex-wrap items-center gap-2">
                <span
                  className={
                    f.severity === 'fail'
                      ? 'rounded-md bg-red-100 px-2 py-0.5 text-xs font-semibold text-red-900 dark:bg-red-500/20 dark:text-red-100'
                      : 'rounded-md bg-amber-100 px-2 py-0.5 text-xs font-semibold text-amber-900 dark:bg-amber-500/20 dark:text-amber-100'
                  }
                >
                  {SEVERITY_LABEL[f.severity]}
                </span>
                <span className="font-medium text-ink dark:text-white">{f.id}</span>
                {f.table && (
                  <span className="text-slate-600 dark:text-slate-300">on {f.table}</span>
                )}
              </div>
              <p className="mt-1 text-slate-800 dark:text-slate-200">{f.message}</p>
            </li>
          ))}
        </ul>
      )}
      {checks.warnings.length > 0 && (
        <div className="mt-3">
          <StatusBadge status="WARNING" />
          <ul className="mt-2 list-disc space-y-1 pl-5 text-sm text-slate-800 dark:text-slate-200">
            {checks.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}
      {checks.scripts.length > 0 && (
        <details className="mt-3 text-sm text-slate-700 dark:text-slate-300">
          <summary className="cursor-pointer font-medium">Scripts that ran</summary>
          <ul className="mt-2 space-y-1">
            {checks.scripts.map((s) => (
              <li key={s.name} className="break-all">
                {s.name} <span className="text-slate-500 dark:text-slate-400">SHA-256 {s.sha256}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </Panel>
  );
}
