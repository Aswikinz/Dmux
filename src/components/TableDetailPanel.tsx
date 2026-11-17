import { forwardRef, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { Button } from '../design/Button';
import { Panel } from '../design/Panel';
import { StatusBadge } from '../design/StatusBadge';
import type { ConnectionFact, Results, TableResult } from '../api/contract';
import { LEVEL_LABELS } from './VerdictBanner';
import {
  LEVEL_MEANING,
  describeChangeDetection,
  describeTls,
  sameDecimal,
  whyThisTable,
} from './tableExplain';

const num = (n: number): string => n.toLocaleString('en-US');

function Heading({ children }: { children: ReactNode }) {
  return (
    <h4 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">
      {children}
    </h4>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-1.5">
      <Heading>{title}</Heading>
      <div className="space-y-1.5 text-sm text-slate-800 dark:text-slate-200">{children}</div>
    </section>
  );
}

function ScrollTable({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div
      role="region"
      aria-label={label}
      tabIndex={0}
      className="max-h-64 overflow-auto rounded-lg border border-black/[0.06] dark:border-white/[0.08]"
    >
      <table className="w-full border-collapse text-left text-sm">{children}</table>
    </div>
  );
}

const TH =
  'sticky top-0 bg-slate-50 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.05em] text-ink-secondary dark:bg-[#2a2a2d] dark:text-slate-400';
const TD = 'border-t border-black/[0.05] px-3 py-1.5 align-top dark:border-white/[0.06]';

interface TableDetailPanelProps {
  table: TableResult;
  onClose: () => void;
}

/**
 * Everything Dmux recorded about how one table was checked, in plain words, for
 * a PASS as much as for a failure.  It never shows a row value: only counts,
 * column names, rules and sums.
 */
export const TableDetailPanel = forwardRef<HTMLHeadingElement, TableDetailPanelProps>(
  function TableDetailPanel({ table: t, onClose }, headingRef) {
    const why = whyThisTable(t);
    const f = t.facts;
    const cd = describeChangeDetection(f);
    const notes = t.columnNotes ?? [];
    const typeChanges = notes.filter((n) => n.kind === 'type_changed');
    const otherNotes = notes.filter((n) => n.kind !== 'type_changed');
    const ignored = t.ignoredColumns ?? [];
    const dataLevel = t.level !== 'structure';

    return (
      <div data-testid="table-detail">
      <Panel className="space-y-5">
        <div className="flex items-start justify-between gap-4">
          <div className="space-y-1">
            <h3
              ref={headingRef}
              tabIndex={-1}
              className="text-[17px] font-semibold tracking-[-0.01em] text-ink focus:outline-none dark:text-white"
            >
              Details for {t.schemaName}.{t.tableName}
            </h3>
            <div className="flex flex-wrap items-center gap-2 text-sm text-slate-700 dark:text-slate-300">
              <StatusBadge status={t.verdict} />
              <span>Checked at the level: {LEVEL_LABELS[t.level]}</span>
            </div>
          </div>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="Close details">
            <X className="h-4 w-4" aria-hidden="true" /> Close
          </Button>
        </div>

        <div
          data-testid="why-this-table"
          className="rounded-xl border border-black/[0.06] bg-slate-50 p-3.5 dark:border-white/[0.08] dark:bg-white/[0.04]"
        >
          <h4 className="text-sm font-semibold text-slate-900 dark:text-slate-100">{why.title}</h4>
          <p className="mt-1 text-sm text-slate-800 dark:text-slate-200">{why.sentence}</p>
        </div>

        <Section title="What was tested">
          <p>
            <span className="font-medium">{LEVEL_LABELS[t.level]}.</span> {LEVEL_MEANING[t.level]}
          </p>
        </Section>

        {dataLevel && (
          <Section title="Rows">
            {t.oldRows !== undefined && t.newRows !== undefined ? (
              <p>
                Old database: <strong>{num(t.oldRows)}</strong> rows. New database:{' '}
                <strong>{num(t.newRows)}</strong> rows.{' '}
                {t.oldRows === t.newRows ? 'The counts are equal.' : 'The counts differ.'}
              </p>
            ) : (
              <p>The row counts were not recorded for this table.</p>
            )}
            {(t.missingRows || t.extraRows || t.changedRows) && (
              <p>
                {num(t.missingRows ?? 0)} missing in the new database, {num(t.extraRows ?? 0)} only in
                the new database, {num(t.changedRows ?? 0)} with changed values.
              </p>
            )}
          </Section>
        )}

        {t.level === 'full-values' && f && (
          <>
            <Section title="Key columns">
              {f.keyColumns.length > 0 ? (
                <p>
                  Rows were matched by{' '}
                  <span className="font-mono text-[13px]">{f.keyColumns.join(', ')}</span>.
                </p>
              ) : (
                <p>This table has no key, so its rows were compared as a counted set.</p>
              )}
            </Section>

            <Section title="Buckets">
              {f.bucketsTotal !== undefined ? (
                <p>
                  Rows are summarised in <strong>{num(f.bucketsTotal)}</strong> buckets and each bucket
                  is compared as one unit. <strong>{num(t.differingBuckets ?? 0)}</strong> differed,{' '}
                  {num(t.missingBuckets ?? 0)} were only in the old database and{' '}
                  {num(t.extraBuckets ?? 0)} only in the new one.
                </p>
              ) : (
                <p>The bucket counts were not recorded for this table.</p>
              )}
            </Section>

            <Section title="Spot check">
              {f.spot?.state === 'agreed' && (
                <p>
                  {num(f.spot.compared ?? 0)} rows were fetched from both databases and compared value by
                  value. {num(f.spot.differing ?? 0)} differed. This agrees with the bucket comparison, so
                  the two methods back each other up.
                </p>
              )}
              {f.spot?.state === 'disagreed' && (
                <p>The spot check disagreed with the bucket comparison: {f.spot.note}</p>
              )}
              {f.spot?.state === 'not_run' && <p>The spot check did not run: {f.spot.note}.</p>}
              {!f.spot && <p>No spot check was recorded for this table.</p>}
            </Section>

            <Section title="Columns compared">
              {f.columns.length > 0 ? (
                <ScrollTable label="Columns compared and the rule for each">
                  <thead>
                    <tr>
                      <th scope="col" className={TH}>Column</th>
                      <th scope="col" className={TH}>How it was compared</th>
                    </tr>
                  </thead>
                  <tbody>
                    {f.columns.map((c) => (
                      <tr key={c.column}>
                        <td className={`${TD} font-mono text-[13px]`}>{c.column}</td>
                        <td className={TD}>{c.rule}</td>
                      </tr>
                    ))}
                  </tbody>
                </ScrollTable>
              ) : (
                <p>No columns were recorded for this table.</p>
              )}
              <p className="text-slate-600 dark:text-slate-400">
                {num(f.columns.length)} {f.columns.length === 1 ? 'column' : 'columns'} compared.
              </p>
            </Section>
          </>
        )}

        {dataLevel && f && f.controlTotals.length > 0 && (
          <Section title="Control totals">
            <ScrollTable label="Control totals on both sides">
              <thead>
                <tr>
                  <th scope="col" className={TH}>Column</th>
                  <th scope="col" className={TH}>Old database</th>
                  <th scope="col" className={TH}>New database</th>
                  <th scope="col" className={TH}>Result</th>
                </tr>
              </thead>
              <tbody>
                {f.controlTotals.map((c) => (
                  <tr key={c.column}>
                    <td className={`${TD} font-mono text-[13px]`}>{c.column}</td>
                    <td className={`${TD} font-mono text-[13px]`}>{c.old}</td>
                    <td className={`${TD} font-mono text-[13px]`}>{c.new}</td>
                    <td className={TD}>{sameDecimal(c.old, c.new) ? 'Match' : 'Differ'}</td>
                  </tr>
                ))}
              </tbody>
            </ScrollTable>
          </Section>
        )}

        {dataLevel && (
          <Section title="Change detection">
            {cd.kind === 'unrecorded' ? (
              <p>No change-detection result was recorded for this table.</p>
            ) : (
              <p>{cd.text}</p>
            )}
          </Section>
        )}

        {(typeChanges.length > 0 || otherNotes.length > 0) && (
          <Section title="Options and type notes">
            <ul className="list-disc space-y-1 pl-5">
              {typeChanges.map((n) => (
                <li key={`${n.column}-${n.kind}`}>
                  <span className="font-mono text-[13px]">{n.column}</span>: {n.note}
                </li>
              ))}
              {otherNotes.map((n) => (
                <li key={`${n.column}-${n.kind}-${n.note}`}>
                  <span className="font-mono text-[13px]">{n.column}</span>: {n.note}
                </li>
              ))}
            </ul>
          </Section>
        )}

        {ignored.length > 0 && (
          <Section title="Ignored columns">
            <ul className="list-disc space-y-1 pl-5">
              {ignored.map((i) => (
                <li key={i.id}>
                  <span className="font-mono text-[13px]">{i.name}</span>: {i.reason}
                </li>
              ))}
            </ul>
          </Section>
        )}

        {(t.reasons?.length ?? 0) > 0 && (
          <Section title="Everything the engine recorded">
            <ul className="list-disc space-y-1 pl-5">
              {t.reasons?.map((r) => (
                <li key={r}>{r}</li>
              ))}
            </ul>
          </Section>
        )}

        {!f && t.level !== 'structure' && (
          <p className="text-sm text-slate-600 dark:text-slate-400">
            Dmux did not record the detailed facts for this table (it may have been reused from an
            earlier run, or compared from a saved file). The counts and reasons above are all that exist.
          </p>
        )}
      </Panel>
      </div>
    );
  },
);

function ConnectionLines({ title, c }: { title: string; c: ConnectionFact }) {
  return (
    <div className="space-y-1">
      <Heading>{title}</Heading>
      <ul className="list-disc space-y-0.5 pl-5 text-sm text-slate-800 dark:text-slate-200">
        <li>{describeTls(c)}.</li>
        <li>
          {c.canWrite
            ? 'The login could write to this database. Dmux never does, but a read-only login is safer.'
            : 'The login is read-only: it cannot change anything in this database.'}
        </li>
        <li>
          {c.superuser
            ? `Superuser login, allowed by you. Reason recorded: ${c.superuserReason ?? 'none given'}`
            : 'Not a superuser login.'}
        </li>
        {(c.serverVersion || c.serverEncoding) && (
          <li>
            Server: PostgreSQL {c.serverVersion || 'unknown'}, text encoding{' '}
            {c.serverEncoding || 'unknown'}.
          </li>
        )}
      </ul>
    </div>
  );
}

/** The run-level "What was checked" section. */
export function WhatWasChecked({ results }: { results: Results }) {
  const facts = results.runFacts;
  const c = results.coverage;
  return (
    <Panel className="space-y-4">
      <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">What was checked</h3>
      <p className="text-sm text-slate-800 dark:text-slate-200" data-testid="coverage-explained">
        {num(c.totalTables)} tables were in scope: {num(c.fullValuesTables)} compared value by value,{' '}
        {num(c.countsTables)} by row counts and totals, {num(c.structureTables)} by structure only, and{' '}
        {num(c.ignoredTables)} ignored with a recorded reason.
      </p>
      {facts ? (
        <>
          <div className="grid gap-4 md:grid-cols-2">
            <ConnectionLines title="Old database" c={facts.oldConnection} />
            <ConnectionLines title="New database" c={facts.newConnection} />
          </div>
          <dl className="space-y-1 text-sm text-slate-800 dark:text-slate-200">
            <div className="flex flex-wrap gap-x-2">
              <dt className="font-medium">Dmux version:</dt>
              <dd>{facts.toolVersion || 'not recorded'}</dd>
            </div>
            <div className="flex flex-wrap gap-x-2">
              <dt className="font-medium">Mapping fingerprint:</dt>
              <dd className="break-all font-mono text-[13px]">{facts.mappingHash || 'not recorded'}</dd>
            </div>
            <div className="flex flex-wrap gap-x-2">
              <dt className="font-medium">Results SHA-256:</dt>
              <dd className="break-all font-mono text-[13px]">{facts.resultsSha256 || 'not recorded'}</dd>
            </div>
          </dl>
          <p className="text-xs text-slate-600 dark:text-slate-400">
            The SHA-256 detects accidental damage to the saved results. To show they were not altered,
            compare it with the value printed on the signed PDF report.
          </p>
        </>
      ) : (
        <p className="text-sm text-slate-600 dark:text-slate-400">
          How each database was connected was not recorded for this run.
        </p>
      )}
    </Panel>
  );
}
