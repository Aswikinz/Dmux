import { useState } from 'react';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { Switch } from '../components/Switch';
import { ErrorCard } from '../design/ErrorCard';
import { StatusBadge } from '../design/StatusBadge';
import { qualifiedVerdictLabel } from '../components/VerdictBanner';
import { coverageLabel } from './ResultsScreen';
import { FileSpreadsheet, FileText } from 'lucide-react';
import type { Engine } from '../api/engine';
import type { ReportFormat, Results } from '../api/contract';

interface ReportScreenProps {
  engine: Engine;
  results: Results;
  onBack: () => void;
  onRestart: () => void;
}

export function ReportScreen({ engine, results, onBack, onRestart }: ReportScreenProps) {
  const [includeKeys, setIncludeKeys] = useState(false);
  const [exporting, setExporting] = useState<ReportFormat | null>(null);
  const [resultMessage, setResultMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function exportReport(format: ReportFormat) {
    setExporting(format);
    setError(null);
    setResultMessage(null);
    try {
      const res = await engine.exportReport({
        runId: results.runId,
        format,
        includeKeys,
      });
      const label = format === 'pdf' ? 'PDF' : 'Excel';
      if (res.ok) {
        setResultMessage(`${label} report saved to ${res.filePath}`);
      } else if (res.cancelled) {
        setResultMessage(`${label} export cancelled. Nothing was saved.`);
      } else {
        setError(res.error ?? 'The report could not be saved.');
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setExporting(null);
    }
  }

  return (
    <div className="space-y-6">
      <ScreenHeader title="Report" subtitle="Save this check as a PDF summary to sign, or as an Excel workbook with the detail." />
      <Panel>
        <div className="flex flex-wrap items-center gap-4">
          <StatusBadge
            status={results.verdict}
            label={qualifiedVerdictLabel(results)}
            className="text-base"
          />
          <p className="text-sm text-slate-700 dark:text-slate-300">
            {coverageLabel(results.coverage)}
          </p>
        </div>
        {results.ignored.length > 0 && (
          <div className="mt-4">
            <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">Ignored</h3>
            <ul className="mt-1 space-y-1 text-sm text-slate-600 dark:text-slate-300">
              {results.ignored.map((item) => (
                <li key={item.id}>
                  <span className="font-medium">{item.name}</span>
                  <span className="ml-2 text-slate-500 dark:text-slate-400">— {item.reason}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
        <div className="mt-4">
          <Switch
            label="Include row keys in the exported report"
            checked={includeKeys}
            onChange={setIncludeKeys}
          />
          <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">
            Off by default. Row values are never exported.
          </p>
        </div>
        <div className="mt-6 flex flex-wrap gap-3">
          <Button
            onClick={() => exportReport('pdf')}
            disabled={exporting !== null}
            variant="secondary"
          >
            <FileText className="h-4 w-4" />
            Export PDF
          </Button>
          <Button
            onClick={() => exportReport('excel')}
            disabled={exporting !== null}
            variant="secondary"
          >
            <FileSpreadsheet className="h-4 w-4" />
            Export Excel
          </Button>
        </div>
        {resultMessage && (
          <p className="mt-3 text-sm text-emerald-700 dark:text-emerald-300">{resultMessage}</p>
        )}
        {error && (
          <ErrorCard title="Export failed" className="mt-3">
            {error}
          </ErrorCard>
        )}
      </Panel>
      <div className="flex items-center justify-between">
        <Button onClick={onBack} variant="secondary">
          Back
        </Button>
        <Button onClick={onRestart} variant="primary" size="lg">
          Start over
        </Button>
      </div>
    </div>
  );
}
