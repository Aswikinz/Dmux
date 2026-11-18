import { useState } from 'react';
import { Button } from '../design/Button';
import { describeError } from '../api/tauriEngine';
import type { Engine } from '../api/engine';
import type { CheckPlan } from '../api/contract';
import { applyChoices, describeApply, planToChoices } from '../screens/checkChoices';

interface CheckChoicesFileProps {
  engine: Engine;
  plan: CheckPlan;
  onPlanChange: (plan: CheckPlan) => void;
}

type Message = { tone: 'ok' | 'info' | 'error'; lines: string[] };

/**
 * Save this run's choices (tables, levels, columns, control totals) to a file,
 * or open a saved file and apply it.  The file holds names only.
 */
export function CheckChoicesFile({ engine, plan, onPlanChange }: CheckChoicesFileProps) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<Message | null>(null);
  const chosen = plan.tables.filter((t) => t.included !== false).length;

  const save = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const r = await engine.saveCheckChoices(planToChoices(plan));
      if (r.status === 'saved') setMessage({ tone: 'ok', lines: [`Saved the choices to ${r.path}.`] });
      else if (r.status === 'cancelled') setMessage({ tone: 'info', lines: ['Not saved: the save window was closed.'] });
      else setMessage({ tone: 'info', lines: ['The browser demo cannot save files. The desktop app can.'] });
    } catch (e) {
      setMessage({ tone: 'error', lines: [`The choices could not be saved: ${describeError(e)}`] });
    } finally {
      setBusy(false);
    }
  };

  const open = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const r = await engine.openCheckChoices();
      if (r.status === 'opened') {
        const { plan: next, report } = applyChoices(plan, r.choices);
        onPlanChange(next);
        const problems =
          report.missingTables.length + report.missingColumns.length + report.keysKept.length + report.notNumbers.length;
        setMessage({ tone: problems > 0 ? 'info' : 'ok', lines: [`Opened ${r.path}.`, ...describeApply(report)] });
      } else if (r.status === 'cancelled') {
        setMessage({ tone: 'info', lines: ['Nothing opened: the open window was closed.'] });
      } else {
        setMessage({ tone: 'info', lines: ['The browser demo cannot open files. The desktop app can.'] });
      }
    } catch (e) {
      setMessage({ tone: 'error', lines: [`The file could not be used: ${describeError(e)}`] });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-2" data-testid="check-choices-file">
      <div className="flex flex-wrap items-center gap-3">
        <div className="mr-auto">
          <h3 className="text-[15px] font-semibold text-ink dark:text-white">Saved choices</h3>
          <p className="text-sm text-slate-600 dark:text-slate-300">
            Save the tables, levels, columns and control totals you chose, and open them again next time
            you check this database. The file holds names only, never logins or data.
          </p>
        </div>
        <Button size="sm" variant="secondary" disabled={busy} onClick={() => void open()}>
          Open saved choices…
        </Button>
        <Button size="sm" variant="secondary" disabled={busy || chosen === 0} onClick={() => void save()}>
          Save these choices…
        </Button>
      </div>
      {message && (
        <div
          role={message.tone === 'error' ? 'alert' : 'status'}
          data-testid="check-choices-message"
          className={
            message.tone === 'error'
              ? 'rounded-lg border border-red-300 bg-red-50 p-3 text-sm text-red-900 dark:border-red-800 dark:bg-red-950/40 dark:text-red-100'
              : 'rounded-lg border border-slate-200 bg-slate-50 p-3 text-sm text-slate-800 dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-100'
          }
        >
          {message.lines.map((l) => (
            <p key={l}>{l}</p>
          ))}
        </div>
      )}
    </div>
  );
}
