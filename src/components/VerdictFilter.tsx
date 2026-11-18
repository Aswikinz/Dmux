import { VERDICT_WORDS } from '../design/StatusBadge';
import { Button } from '../design/Button';
import type { Verdict } from '../api/contract';

/** Gravest first, so the order of the boxes never depends on the data. */
export const VERDICT_ORDER: Verdict[] = [
  'TOOL_FAULT',
  'FAIL',
  'WIDELY_DIFFERENT',
  'CHANGED_DURING_CHECK',
  'NEEDS_REVIEW',
  'WARNING',
  'PASS',
];

interface VerdictFilterProps {
  /** How many results carry each verdict (only verdicts present get a box). */
  counts: Partial<Record<Verdict, number>>;
  hidden: Set<Verdict>;
  onChange: (hidden: Set<Verdict>) => void;
}

/**
 * Choose which results the lists below show, by verdict.  Everything is shown
 * until the user hides something, and the line under the boxes always says
 * how many results are hidden, so nothing disappears silently.  The overall
 * verdict at the top is never filtered.
 */
export function VerdictFilter({ counts, hidden, onChange }: VerdictFilterProps) {
  const present = VERDICT_ORDER.filter((v) => (counts[v] ?? 0) > 0);
  if (present.length < 2) return null;
  const hiddenCount = present
    .filter((v) => hidden.has(v))
    .reduce((n, v) => n + (counts[v] ?? 0), 0);
  const toggle = (v: Verdict, show: boolean) => {
    const next = new Set(hidden);
    if (show) next.delete(v);
    else next.add(v);
    onChange(next);
  };
  return (
    <div className="flex flex-col gap-2" data-testid="verdict-filter">
      <fieldset className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <legend className="float-left mr-1 text-sm font-medium text-slate-700 dark:text-slate-200">
          Show:
        </legend>
        {present.map((v) => (
          <label key={v} className="flex items-center gap-1.5 text-sm text-slate-700 dark:text-slate-200">
            <input
              type="checkbox"
              className="h-4 w-4 accent-accent-600"
              checked={!hidden.has(v)}
              onChange={(e) => toggle(v, e.target.checked)}
            />
            <span className="font-semibold">{VERDICT_WORDS[v]}</span>
            <span className="text-slate-500 dark:text-slate-400">({counts[v]})</span>
          </label>
        ))}
        {(counts.WARNING ?? 0) > 0 && (
          <Button
            size="sm"
            variant="secondary"
            onClick={() => toggle('WARNING', hidden.has('WARNING'))}
          >
            {hidden.has('WARNING') ? 'Show warnings' : 'Hide warnings'}
          </Button>
        )}
      </fieldset>
      <p role="status" className="text-xs text-slate-600 dark:text-slate-400">
        {hiddenCount === 0
          ? 'Showing every result.'
          : `${hiddenCount === 1 ? '1 result is' : `${hiddenCount} results are`} hidden by this filter. The overall verdict above still counts them.`}
      </p>
    </div>
  );
}
