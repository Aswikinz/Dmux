import { useCallback, useEffect, useMemo, useState } from 'react';
import { createColumnHelper, type ColumnDef } from '@tanstack/react-table';
import { FolderOpen, ScrollText } from 'lucide-react';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { StatusBadge } from '../design/StatusBadge';
import { ErrorCard } from '../design/ErrorCard';
import { DataTable } from '../components/DataTable';
import { qualifiedVerdictLabel } from '../components/VerdictBanner';
import type { Engine } from '../api/engine';
import type { HistoryEntry, Results } from '../api/contract';

interface HistoryScreenProps {
  engine: Engine;
  /** Show verified results on the Results screen. */
  onOpenResults: (results: Results) => void;
}

/** A fixed, unambiguous date: "2026-11-16 09:48 UTC". */
export function formatWhen(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return 'Unknown';
  }
  return `${date.toISOString().slice(0, 16).replace('T', ' ')} UTC`;
}

function matchesQuery(row: HistoryEntry, q: string): boolean {
  return [row.verdict, row.coverage, row.oldLabel, row.newLabel, formatWhen(row.endedAt)]
    .join(' ')
    .toLowerCase()
    .includes(q);
}

function describeThrown(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function HistoryScreen({ engine, onOpenResults }: HistoryScreenProps) {
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [log, setLog] = useState<{ id: string; text: string } | null>(null);
  const [logNotice, setLogNotice] = useState<string | null>(null);
  const [confirmingPurge, setConfirmingPurge] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setEntries(await engine.listHistory());
    } catch (e) {
      setError(describeThrown(e));
      setEntries([]);
    }
  }, [engine]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const openEntry = useCallback(
    async (id: string) => {
      setBusy(true);
      setError(null);
      setNotice(null);
      try {
        onOpenResults(await engine.openHistoryEntry(id));
      } catch (e) {
        setError(describeThrown(e));
      } finally {
        setBusy(false);
      }
    },
    [engine, onOpenResults],
  );

  const showLog = useCallback(
    async (id: string) => {
      setError(null);
      setLogNotice(null);
      try {
        setLog({ id, text: await engine.getHistoryLog(id) });
      } catch (e) {
        setLog(null);
        setError(describeThrown(e));
      }
    },
    [engine],
  );

  async function openFile() {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await engine.openResultsFile();
      if (result.ok && result.results) {
        onOpenResults(result.results);
      } else if (result.cancelled) {
        setNotice('Nothing was opened.');
      } else {
        setError(result.error ?? 'That file could not be opened.');
      }
    } catch (e) {
      setError(describeThrown(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveLog() {
    if (!log) return;
    setLogNotice(null);
    setError(null);
    try {
      const res = await engine.saveHistoryLog(log.id);
      if (res.ok) {
        setLogNotice(`Run log saved to ${res.filePath}`);
      } else if (res.cancelled) {
        setLogNotice('Saving cancelled. Nothing was saved.');
      } else {
        setError(res.error ?? 'The run log could not be saved.');
      }
    } catch (e) {
      setError(describeThrown(e));
    }
  }

  async function purge() {
    setBusy(true);
    setError(null);
    try {
      const removed = await engine.purgeHistory();
      setConfirmingPurge(false);
      setLog(null);
      setNotice(
        removed === 1
          ? 'Deleted 1 stored check and its results file.'
          : `Deleted ${removed} stored checks and their results files.`,
      );
      await refresh();
    } catch (e) {
      setError(describeThrown(e));
    } finally {
      setBusy(false);
    }
  }

  const columns = useMemo(() => {
    const col = createColumnHelper<HistoryEntry>();
    return [
      col.accessor('endedAt', {
        header: 'Finished',
        cell: ({ getValue }) => formatWhen(getValue()),
      }),
      col.accessor('verdict', {
        header: 'Verdict',
        cell: ({ row }) => (
          <StatusBadge
            status={row.original.verdict}
            label={qualifiedVerdictLabel(row.original)}
          />
        ),
      }),
      col.accessor('coverage', {
        header: 'Coverage',
        cell: ({ getValue }) => <span className="text-xs">{getValue()}</span>,
      }),
      col.display({
        id: 'profiles',
        header: 'Databases',
        cell: ({ row }) => (
          <span className="text-xs">
            {row.original.oldLabel || 'Old'} to {row.original.newLabel || 'New'}
          </span>
        ),
      }),
      col.display({
        id: 'actions',
        header: 'Actions',
        cell: ({ row }) => (
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={busy}
              onClick={() => void openEntry(row.original.id)}
              aria-label={`Open results from ${formatWhen(row.original.endedAt)}`}
            >
              Open results
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void showLog(row.original.id)}
              aria-label={`Show the run log from ${formatWhen(row.original.endedAt)}`}
            >
              Run log
            </Button>
          </div>
        ),
      }),
    ] as ColumnDef<HistoryEntry>[];
  }, [busy, openEntry, showLog]);

  const empty = entries !== null && entries.length === 0;

  return (
    <div className="space-y-6">
      <ScreenHeader
        title="History"
        subtitle="Every finished check is kept on this computer. Open one to see its results again, or open a results file someone sent you."
        actions={
          <Button variant="secondary" onClick={() => void openFile()} disabled={busy}>
            <FolderOpen className="h-4 w-4" aria-hidden="true" />
            Open a results file
          </Button>
        }
      />
      {error && <ErrorCard title="That could not be shown">{error}</ErrorCard>}
      {notice && (
        <p className="text-sm text-slate-700 dark:text-slate-300" role="status">
          {notice}
        </p>
      )}
      <Panel>
        {entries === null && (
          <p className="py-6 text-center text-slate-600 dark:text-slate-300">Loading history…</p>
        )}
        {empty && (
          <p className="py-6 text-center text-slate-600 dark:text-slate-300">
            No finished checks yet. When a check finishes, it appears here.
          </p>
        )}
        {entries !== null && entries.length > 0 && (
          <DataTable
            data={entries}
            columns={columns}
            rowHeight={56}
            filterPlaceholder="Search the history"
            filterFn={matchesQuery}
            getRowId={(row) => row.id}
          />
        )}
      </Panel>
      {log && (
        <Panel>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h3 className="flex items-center gap-2 text-base font-semibold text-slate-900 dark:text-slate-100">
              <ScrollText className="h-4 w-4" aria-hidden="true" />
              Run log
            </h3>
            <div className="flex gap-2">
              <Button size="sm" variant="secondary" onClick={() => void saveLog()}>
                Save run log
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setLog(null)}>
                Close
              </Button>
            </div>
          </div>
          <pre
            className="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-xl border border-black/[0.06] bg-black/[0.03] p-3 text-xs text-slate-800 dark:border-white/[0.08] dark:bg-white/[0.05] dark:text-slate-200"
            tabIndex={0}
            aria-label="Run log text"
            data-testid="run-log-text"
          >
            {log.text}
          </pre>
          {logNotice && (
            <p className="mt-2 text-sm text-emerald-700 dark:text-emerald-300" role="status">
              {logNotice}
            </p>
          )}
        </Panel>
      )}
      <Panel>
        <h3 className="text-base font-semibold text-slate-900 dark:text-slate-100">
          Delete local history
        </h3>
        <p className="mt-1 text-sm text-slate-700 dark:text-slate-300">
          This deletes every stored results file and the history list on this computer. Your saved
          connections are kept. Reports and files you saved elsewhere are not touched.
        </p>
        {!confirmingPurge ? (
          <Button
            className="mt-3"
            variant="secondary"
            disabled={busy || empty}
            onClick={() => setConfirmingPurge(true)}
          >
            Delete all history
          </Button>
        ) : (
          <div className="mt-3 flex flex-wrap items-center gap-3" role="group" aria-label="Confirm deleting history">
            <span className="text-sm font-medium text-red-800 dark:text-red-300">
              Delete all stored checks? This cannot be undone.
            </span>
            <Button variant="danger" size="sm" disabled={busy} onClick={() => void purge()}>
              Yes, delete everything
            </Button>
            <Button variant="secondary" size="sm" onClick={() => setConfirmingPurge(false)}>
              Keep my history
            </Button>
          </div>
        )}
      </Panel>
    </div>
  );
}
