import { Button } from '../design/Button';
import type { ReportLeaveOut, Results } from '../api/contract';
import { kindCounts, resultItems } from '../screens/ResultsScreen';

export const LEAVE_NOTHING_OUT: ReportLeaveOut = {
  structure: false,
  other: false,
  warnings: false,
  passing: false,
};

/** How many of each part a report could hold, for the boxes' counts. */
export function reportPartCounts(results: Results) {
  const kinds = kindCounts(results);
  const items = resultItems(results);
  return {
    data: kinds.data ?? 0,
    structure: kinds.structure ?? 0,
    other: kinds.other ?? 0,
    warnings: items.filter((i) => i.verdict === 'WARNING').length + results.warnings.length,
    passing: results.tableResults.filter((t) => t.verdict === 'PASS').length,
    notMapped: results.tableResults.filter((t) => t.kind === 'not_mapped').length,
  };
}

interface ReportContentsProps {
  results: Results;
  leaveOut: ReportLeaveOut;
  onChange: (leaveOut: ReportLeaveOut) => void;
}

/**
 * Choose what goes into the exported report.  Data differences are always
 * included and come first; every other part can be left out.  The report
 * names whatever was left out on its first page, and its verdict and coverage
 * still count everything.
 */
export function ReportContents({ results, leaveOut, onChange }: ReportContentsProps) {
  const n = reportPartCounts(results);
  const parts: { key: keyof ReportLeaveOut; label: string; count: number }[] = [
    { key: 'structure', label: 'Structure differences', count: n.structure },
    { key: 'other', label: 'Failures for other reasons', count: n.other },
    { key: 'warnings', label: 'Warnings', count: n.warnings },
    { key: 'passing', label: 'Tables that passed', count: n.passing },
  ];
  const onlyData = parts.every((p) => leaveOut[p.key]);
  const anything = parts.some((p) => leaveOut[p.key]);
  return (
    <fieldset className="mt-4 rounded-xl border border-slate-200 p-4 dark:border-slate-700" data-testid="report-contents">
      <legend className="px-1 text-sm font-semibold text-slate-800 dark:text-slate-100">
        What goes into the report
      </legend>
      <ul className="space-y-2 text-sm">
        <li>
          <label className="flex items-center gap-2 font-medium text-slate-800 dark:text-slate-100">
            <input type="checkbox" className="h-4 w-4 accent-accent-600" checked disabled />
            Data differences ({n.data})
            <span className="font-normal text-slate-500 dark:text-slate-400">
              — always included, first
            </span>
          </label>
        </li>
        <li>
          <label className="flex items-center gap-2 font-medium text-slate-800 dark:text-slate-100">
            <input type="checkbox" className="h-4 w-4 accent-accent-600" checked disabled />
            Not mapped ({n.notMapped})
            <span className="font-normal text-slate-500 dark:text-slate-400">
              — always included: a table missing from the new database is data that did not arrive
            </span>
          </label>
        </li>
        {parts.map((p) => (
          <li key={p.key}>
            <label className="flex items-center gap-2 text-slate-700 dark:text-slate-200">
              <input
                type="checkbox"
                className="h-4 w-4 accent-accent-600"
                checked={!leaveOut[p.key]}
                onChange={(e) => onChange({ ...leaveOut, [p.key]: !e.target.checked })}
              />
              {p.label} ({p.count})
            </label>
          </li>
        ))}
      </ul>
      <div className="mt-3 flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="secondary"
          disabled={onlyData}
          onClick={() => onChange({ structure: true, other: true, warnings: true, passing: true })}
        >
          Only data differences
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={!anything}
          onClick={() => onChange(LEAVE_NOTHING_OUT)}
        >
          Include everything
        </Button>
      </div>
      <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
        Anything you leave out is named on the report&apos;s first page. The verdict, coverage and
        counts still include it, and the run log always lists everything.
      </p>
    </fieldset>
  );
}
