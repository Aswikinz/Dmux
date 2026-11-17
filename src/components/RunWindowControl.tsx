import { useState } from 'react';
import { Switch } from './Switch';
import { fieldClasses } from '../design/Input';

interface RunWindowControlProps {
  /** The window as "HH:MM-HH:MM" (local time), or undefined when off. */
  value: string | undefined;
  onChange: (value: string | undefined) => void;
}

const DEFAULT_START = '22:00';
const DEFAULT_END = '05:00';

/** Split "22:00-05:00" into its two times; undefined parts fall back to the defaults. */
export function splitRunWindow(value: string | undefined): { start: string; end: string } {
  const m = /^(\d{1,2}:\d{2})-(\d{1,2}:\d{2})$/.exec(value ?? '');
  const pad = (t: string) => (t.length === 4 ? `0${t}` : t);
  return m ? { start: pad(m[1]), end: pad(m[2]) } : { start: DEFAULT_START, end: DEFAULT_END };
}

/**
 * An optional daily window. Outside it the run waits between tables and carries
 * on by itself when the window opens. Off by default.
 */
export function RunWindowControl({ value, onChange }: RunWindowControlProps) {
  const on = value !== undefined;
  // Keep the last times while the switch is off, so toggling does not reset them.
  const [times, setTimes] = useState(() => splitRunWindow(value));
  const shown = on ? splitRunWindow(value) : times;

  function update(next: { start: string; end: string }) {
    setTimes(next);
    onChange(next.start && next.end ? `${next.start}-${next.end}` : undefined);
  }

  const same = on && shown.start === shown.end;

  return (
    <div
      className="flex flex-col gap-3 rounded-xl border border-black/[0.08] p-4 dark:border-white/10"
      data-testid="run-window"
    >
      <Switch
        label="Only run during certain hours"
        checked={on}
        onChange={(checked) => (checked ? update(times) : onChange(undefined))}
      />
      <p className="text-sm text-slate-600 dark:text-slate-300">
        Outside these hours Dmux waits between tables and carries on by itself when they start
        again. The times are on this computer&apos;s clock. A window such as 22:00 to 05:00 runs
        through the night.
      </p>
      {on && (
        <div className="flex flex-wrap items-end gap-4">
          <label className="flex flex-col gap-1.5 text-[13px] font-medium text-slate-700 dark:text-slate-300">
            Run from
            <input
              type="time"
              required
              value={shown.start}
              onChange={(e) => update({ ...shown, start: e.target.value })}
              className={`${fieldClasses} w-32`}
            />
          </label>
          <label className="flex flex-col gap-1.5 text-[13px] font-medium text-slate-700 dark:text-slate-300">
            Until
            <input
              type="time"
              required
              value={shown.end}
              onChange={(e) => update({ ...shown, end: e.target.value })}
              className={`${fieldClasses} w-32`}
            />
          </label>
          {same && (
            <p role="alert" className="text-sm text-red-700 dark:text-red-300">
              The start and end times must be different.
            </p>
          )}
        </div>
      )}
    </div>
  );
}
