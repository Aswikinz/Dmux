import { useEffect, useState } from 'react';
import { describeError } from '../api/tauriEngine';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { Switch } from '../components/Switch';
import { ErrorCard } from '../design/ErrorCard';
import { StatusBadge } from '../design/StatusBadge';
import { qualifiedVerdictLabel } from '../components/VerdictBanner';
import { coverageLabel } from './ResultsScreen';
import { IgnoredItems } from '../components/IgnoredItems';
import { LEAVE_NOTHING_OUT, ReportContents } from '../components/ReportContents';
import { FileCode2, FileSpreadsheet, FileText } from 'lucide-react';
import type { Engine } from '../api/engine';
import type { ReportFormat, ReportLeaveOut, Results } from '../api/contract';

const FORMAT_LABEL: Record<ReportFormat, string> = {
  pdf: 'PDF',
  excel: 'Excel',
  html: 'HTML',
};

interface ReportScreenProps {
  engine: Engine;
  results: Results;
  onBack: () => void;
  onRestart: () => void;
}

export function ReportScreen({ engine, results, onBack, onRestart }: ReportScreenProps) {
  const [includeKeys, setIncludeKeys] = useState(false);
  // The box starts from the Settings default (off unless the user changed it);
  // it stays an explicit choice for every export.
  useEffect(() => {
    let mounted = true;
    engine
      .getSettings()
      .then((s) => {
        if (mounted) setIncludeKeys(s.includeKeysDefault);
      })
      .catch(() => {});
    return () => {
      mounted = false;
    };
  }, [engine]);
  const [leaveOut, setLeaveOut] = useState<ReportLeaveOut>(LEAVE_NOTHING_OUT);
  const [exporting, setExporting] = useState<ReportFormat | null>(null);
  const [resultMessage, setResultMessage] = useState<{ text: string; saved: boolean } | null>(
    null,
  );
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
        leaveOut,
      });
      const label = FORMAT_LABEL[format];
      if (res.ok) {
        setResultMessage({ text: `${label} report saved to ${res.filePath}`, saved: true });
      } else if (res.cancelled) {
        setResultMessage({ text: `${label} export cancelled. Nothing was saved.`, saved: false });
      } else if (res.demoOnly) {
        setResultMessage({
          text: `This is the browser demo, so no ${label} file was saved. In the Dmux desktop app, you choose where to save it.`,
          saved: false,
        });
      } else {
        setError(res.error ?? 'The report could not be saved.');
      }
    } catch (e) {
      setError(describeError(e));
    } finally {
      setExporting(null);
    }
  }

  return (
    <div className="space-y-6">
      <ScreenHeader
        title="Report"
        subtitle="Save this check as a PDF summary to sign, an Excel workbook with the detail, or a web page you can search and filter."
      />
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
        <IgnoredItems ignored={results.ignored} />
        <ReportContents results={results} leaveOut={leaveOut} onChange={setLeaveOut} />
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
          <Button
            onClick={() => exportReport('html')}
            disabled={exporting !== null}
            variant="secondary"
          >
            <FileCode2 className="h-4 w-4" />
            Export HTML
          </Button>
        </div>
        <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
          Every report shows when the check ran, when the report was made and which databases were
          compared. The HTML report is one file that opens in any browser, works offline, and lets
          you search, filter and sort the tables.
        </p>
        {resultMessage && (
          <p
            className={
              resultMessage.saved
                ? 'mt-3 text-sm text-emerald-700 dark:text-emerald-300'
                : 'mt-3 text-sm text-slate-700 dark:text-slate-300'
            }
          >
            {resultMessage.text}
          </p>
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
