import { VERDICT_WORDS } from '../design/StatusBadge';
import { Button } from '../design/Button';
import { ISSUE_KIND_LABELS } from '../api/contract';
import type { IssueKind, Verdict } from '../api/contract';

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

/** Data differences first: they are what a migration sign-off is about. */
export const KIND_ORDER: IssueKind[] = ['data', 'structure', 'other'];

interface VerdictFilterProps {
  /** How many results carry each verdict (only verdicts present get a box). */
  counts: Partial<Record<Verdict, number>>;
  hidden: Set<Verdict>;
  onChange: (hidden: Set<Verdict>) => void;
  /** How many failures are of each kind (only kinds present get a box). */
  kindCounts?: Partial<Record<IssueKind, number>>;
  hiddenKinds?: Set<IssueKind>;
  onKindsChange?: (hidden: Set<IssueKind>) => void;
  /** How many results the two filters hide together. */
  hiddenTotal: number;
}

/**
 * Choose which results the lists below show: by verdict, and by kind of
 * problem (data differences, structure differences, other reasons).
 * Everything is shown until the user hides something, and the line under the
 * boxes always says how many results are hidden, so nothing disappears
 * silently.  The overall verdict at the top is never filtered.
 */
export function VerdictFilter({
  counts,
  hidden,
  onChange,
  kindCounts = {},
  hiddenKinds = new Set(),
  onKindsChange = () => {},
  hiddenTotal,
}: VerdictFilterProps) {
  const present = VERDICT_ORDER.filter((v) => (counts[v] ?? 0) > 0);
  const kinds = KIND_ORDER.filter((k) => (kindCounts[k] ?? 0) > 0);
  if (present.length < 2 && kinds.length < 2) return null;

  const toggle = (v: Verdict, show: boolean) => {
    const next = new Set(hidden);
    if (show) next.delete(v);
    else next.add(v);
    onChange(next);
  };
  const toggleKind = (k: IssueKind, show: boolean) => {
    const next = new Set(hiddenKinds);
    if (show) next.delete(k);
    else next.add(k);
    onKindsChange(next);
  };
  const onlyData = kinds.length > 1 && kinds.every((k) => (k === 'data') !== hiddenKinds.has(k));

  return (
    <div className="flex flex-col gap-3" data-testid="verdict-filter">
      {kinds.length > 1 && (
        <fieldset className="flex flex-wrap items-center gap-x-4 gap-y-2" data-testid="kind-filter">
          <legend className="float-left mr-1 text-sm font-medium text-slate-700 dark:text-slate-200">
            Kind of problem:
          </legend>
          {kinds.map((k) => (
            <label key={k} className="flex items-center gap-1.5 text-sm text-slate-700 dark:text-slate-200">
              <input
                type="checkbox"
                className="h-4 w-4 accent-accent-600"
                checked={!hiddenKinds.has(k)}
                onChange={(e) => toggleKind(k, e.target.checked)}
              />
              <span className="font-semibold">{ISSUE_KIND_LABELS[k]}</span>
              <span className="text-slate-500 dark:text-slate-400">({kindCounts[k]})</span>
            </label>
          ))}
          <Button
            size="sm"
            variant="secondary"
            onClick={() =>
              onKindsChange(onlyData ? new Set() : new Set(kinds.filter((k) => k !== 'data')))
            }
          >
            {onlyData ? 'Show every kind' : 'Only data differences'}
          </Button>
        </fieldset>
      )}
      {present.length > 1 && (
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
      )}
      <p role="status" className="text-xs text-slate-600 dark:text-slate-400">
        {hiddenTotal === 0
          ? 'Showing every result.'
          : `${hiddenTotal === 1 ? '1 result is' : `${hiddenTotal} results are`} hidden by this filter. The overall verdict above still counts them.`}
      </p>
    </div>
  );
}
