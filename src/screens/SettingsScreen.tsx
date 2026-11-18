import { useCallback, useEffect, useState } from 'react';
import { describeError } from '../api/tauriEngine';
import { Panel } from '../design/Panel';
import { Select } from '../design/Select';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { ErrorCard } from '../design/ErrorCard';
import { useTheme } from '../design/theme';
import { Switch } from '../components/Switch';
import { HookWarning } from '../components/HookWarning';
import type { Engine } from '../api/engine';
import type { AppSettings } from '../api/contract';

interface SettingsScreenProps {
  engine: Engine;
}

const MODES: { id: 'light' | 'dark' | 'system'; label: string }[] = [
  { id: 'system', label: 'Match my computer' },
  { id: 'light', label: 'Light' },
  { id: 'dark', label: 'Dark' },
];

const PYTHON_SOURCE_TEXT: Record<AppSettings['pythonSource'], string> = {
  chosen: 'the program you chose',
  bundled: 'the Python that comes with Dmux',
  auto: 'found on this computer',
};

export function SettingsScreen({ engine }: SettingsScreenProps) {
  const { mode, setMode, reducedEffects, toggleReducedEffects } = useTheme();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [timeoutText, setTimeoutText] = useState('60');

  const apply = useCallback((s: AppSettings) => {
    setSettings(s);
    setTimeoutText(String(s.timeoutSecs));
  }, []);

  useEffect(() => {
    let mounted = true;
    engine
      .getSettings()
      .then((s) => {
        if (mounted) apply(s);
      })
      .catch((e: unknown) => {
        if (mounted) setError(describeError(e));
      });
    return () => {
      mounted = false;
    };
  }, [engine, apply]);

  async function run(action: () => Promise<AppSettings>) {
    setError(null);
    try {
      apply(await action());
    } catch (e) {
      setError(describe(e));
    }
  }

  const timeoutValue = Number(timeoutText);
  const timeoutValid = Number.isInteger(timeoutValue) && timeoutValue >= 1 && timeoutValue <= 600;

  return (
    <div className="space-y-6">
      <ScreenHeader
        title="Settings"
        subtitle="How Dmux looks, how it runs checks, what reports include, and optional custom checks."
      />
      {error && <ErrorCard title="Could not change the setting">{error}</ErrorCard>}

      <Panel>
        <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">Appearance</h3>
        <fieldset className="mt-3">
          <legend className="text-[13px] font-medium text-slate-700 dark:text-slate-300">Colour scheme</legend>
          <div className="mt-2 flex flex-wrap gap-4">
            {MODES.map((m) => (
              <label key={m.id} className="inline-flex cursor-pointer items-center gap-2 text-sm text-slate-800 dark:text-slate-200">
                <input
                  type="radio"
                  name="appearance"
                  checked={mode === m.id}
                  onChange={() => setMode(m.id)}
                  className="h-4 w-4 accent-accent-600"
                />
                {m.label}
              </label>
            ))}
          </div>
        </fieldset>
        <div className="mt-4">
          <Switch label="Reduce visual effects" checked={reducedEffects} onChange={() => toggleReducedEffects()} />
        </div>
      </Panel>

      {settings && (
        <Panel>
          <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">Run defaults</h3>
          <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">
            These are where each new check starts. You can still change them for a single run.
          </p>
          <Select
            id="starting-level"
            label="Starting check level for new plans"
            className="mt-4 max-w-md"
            value={settings.startingLevel}
            onChange={(e) =>
              void run(() =>
                engine.updateSettings({ startingLevel: e.target.value as AppSettings['startingLevel'] }),
              )
            }
            options={[
              { value: 'suggested', label: "Dmux's suggestion for each table" },
              { value: 'full-values', label: 'Full values for every table' },
              { value: 'counts', label: 'Counts and totals for every table' },
              { value: 'structure', label: 'Structure only for every table' },
            ]}
          />
          <div className="mt-4 flex flex-wrap gap-6">
            <CountField
              id="old-parallelism"
              label="Scans at once on the old database"
              value={settings.oldParallelism}
              onCommit={(n) => void run(() => engine.updateSettings({ oldParallelism: n }))}
            />
            <CountField
              id="new-parallelism"
              label="Scans at once on the new database"
              value={settings.newParallelism}
              onCommit={(n) => void run(() => engine.updateSettings({ newParallelism: n }))}
            />
          </div>
          <p className="mt-2 text-xs text-slate-600 dark:text-slate-400">
            More scans finish sooner but put more load on the database. Use a low number on a busy production server.
          </p>
        </Panel>
      )}

      {settings && (
        <Panel>
          <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">Reports</h3>
          <div className="mt-3">
            <Switch
              label="Start with “include row keys” ticked"
              checked={settings.includeKeysDefault}
              onChange={(on) => void run(() => engine.updateSettings({ includeKeysDefault: on }))}
            />
          </div>
          <p className="mt-2 text-sm text-slate-600 dark:text-slate-300">
            Row keys name the rows that differ, which can be sensitive. This is off unless you turn it on, and the
            Report screen still lets you decide for each export.
          </p>
        </Panel>
      )}

      <Panel>
        <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">Custom checks</h3>
        <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">
          A custom check is a small Python script you or your team wrote. After Dmux compares the databases,
          it hands each script a summary of the results (counts and names, never passwords or row values),
          and the script can add its own findings. Custom checks are off unless you turn them on here.
        </p>
        <HookWarning className="mt-4">
          Dmux refuses scripts that other users on this computer could change.
        </HookWarning>

        {settings && (
          <div className="mt-5 space-y-5">
            <Switch
              label="Run custom checks after each comparison"
              checked={settings.customChecksEnabled}
              onChange={(on) => void run(() => engine.updateSettings({ customChecksEnabled: on }))}
            />

            <div>
              <div className="text-[13px] font-medium text-slate-700 dark:text-slate-300">Scripts folder</div>
              <p className="mt-1 break-all text-sm text-slate-900 dark:text-slate-100" data-testid="hooks-folder">
                {settings.hooksFolder ?? 'No folder chosen yet.'}
              </p>
              <div className="mt-2 flex flex-wrap gap-2">
                <Button size="sm" onClick={() => void run(() => engine.chooseHooksFolder())}>
                  Choose folder…
                </Button>
                {settings.hooksFolder && (
                  <Button size="sm" variant="ghost" onClick={() => void run(() => engine.clearHooksFolder())}>
                    Remove folder
                  </Button>
                )}
              </div>
              <FolderList settings={settings} />
            </div>

            <div>
              <div className="text-[13px] font-medium text-slate-700 dark:text-slate-300">Python</div>
              <p className="mt-1 break-all text-sm text-slate-900 dark:text-slate-100" data-testid="python-found">
                {settings.pythonFound
                  ? `${settings.pythonFound} (${PYTHON_SOURCE_TEXT[settings.pythonSource]})`
                  : 'No Python was found on this computer. Install Python 3, or choose the program yourself.'}
              </p>
              <div className="mt-2 flex flex-wrap gap-2">
                <Button size="sm" onClick={() => void run(() => engine.choosePython())}>
                  Choose Python…
                </Button>
                {settings.pythonPath && (
                  <Button size="sm" variant="ghost" onClick={() => void run(() => engine.clearPython())}>
                    Find it automatically
                  </Button>
                )}
              </div>
            </div>

            <div className="max-w-xs">
              <label htmlFor="hook-timeout" className="text-[13px] font-medium text-slate-700 dark:text-slate-300">
                Time limit for each script (seconds)
              </label>
              <div className="mt-1 flex items-center gap-2">
                <input
                  id="hook-timeout"
                  type="number"
                  min={1}
                  max={600}
                  value={timeoutText}
                  onChange={(e) => setTimeoutText(e.target.value)}
                  onBlur={() => {
                    if (timeoutValid && timeoutValue !== settings.timeoutSecs) {
                      void run(() => engine.updateSettings({ timeoutSecs: timeoutValue }));
                    }
                  }}
                  aria-invalid={!timeoutValid}
                  aria-describedby="hook-timeout-help"
                  className="h-9 w-28 rounded-[10px] border border-black/[0.1] bg-white px-3 text-sm text-ink focus:border-accent-500 focus:outline-none focus:ring-4 focus:ring-accent-500/20 dark:border-white/10 dark:bg-white/[0.06] dark:text-slate-100"
                />
              </div>
              <p
                id="hook-timeout-help"
                className={
                  timeoutValid
                    ? 'mt-1 text-xs text-slate-600 dark:text-slate-400'
                    : 'mt-1 text-xs font-medium text-red-700 dark:text-red-300'
                }
              >
                {timeoutValid
                  ? 'A script that takes longer is stopped and the run shows a warning.'
                  : 'Enter a whole number from 1 to 600.'}
              </p>
            </div>
          </div>
        )}
      </Panel>

      {settings && (
        <Panel>
          <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">About</h3>
          <dl className="mt-3 space-y-2 text-sm text-slate-800 dark:text-slate-200">
            <div>
              <dt className="text-[13px] font-medium text-slate-700 dark:text-slate-300">Version</dt>
              <dd data-testid="about-version">{settings.appVersion}</dd>
            </div>
            <div>
              <dt className="text-[13px] font-medium text-slate-700 dark:text-slate-300">
                Where Dmux keeps its data
              </dt>
              <dd className="break-all" data-testid="about-data-dir">{settings.dataDir}</dd>
            </div>
            <div>
              <dt className="text-[13px] font-medium text-slate-700 dark:text-slate-300">Licence</dt>
              <dd>Dmux is open source under the {settings.licence} licence.</dd>
            </div>
          </dl>
        </Panel>
      )}
    </div>
  );
}

function CountField({
  id,
  label,
  value,
  onCommit,
}: {
  id: string;
  label: string;
  value: number;
  onCommit: (n: number) => void;
}) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  const n = Number(text);
  const valid = Number.isInteger(n) && n >= 1 && n <= 32;
  return (
    <div>
      <label htmlFor={id} className="text-[13px] font-medium text-slate-700 dark:text-slate-300">
        {label}
      </label>
      <input
        id={id}
        type="number"
        min={1}
        max={32}
        value={text}
        aria-invalid={!valid}
        aria-describedby={`${id}-help`}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          if (valid && n !== value) onCommit(n);
        }}
        className="mt-1 block h-9 w-28 rounded-[10px] border border-black/[0.1] bg-white px-3 text-sm text-ink focus:border-accent-500 focus:outline-none focus:ring-4 focus:ring-accent-500/20 dark:border-white/10 dark:bg-white/[0.06] dark:text-slate-100"
      />
      <p
        id={`${id}-help`}
        className={valid ? 'sr-only' : 'mt-1 text-xs font-medium text-red-700 dark:text-red-300'}
      >
        {valid ? 'A whole number from 1 to 32.' : 'Enter a whole number from 1 to 32.'}
      </p>
    </div>
  );
}

function FolderList({ settings }: { settings: AppSettings }) {
  const scan = settings.folderScan;
  if (!scan) return null;
  if (scan.folderProblem) {
    return (
      <p role="alert" className="mt-3 text-sm font-medium text-red-800 dark:text-red-300">
        Dmux will not run anything from this folder: {scan.folderProblem}.
      </p>
    );
  }
  if (scan.scripts.length === 0) {
    return (
      <p className="mt-3 text-sm text-slate-700 dark:text-slate-300">
        This folder has no Python scripts (files ending in .py).
      </p>
    );
  }
  return (
    <ul className="mt-3 space-y-1.5" aria-label="Scripts in the folder">
      {scan.scripts.map((s) => (
        <li key={s.name} className="text-sm text-slate-900 dark:text-slate-100">
          <span className="break-all font-medium">{s.name}</span>
          {s.usable ? (
            <span className="ml-2 text-emerald-800 dark:text-emerald-300">Will run</span>
          ) : (
            <span className="ml-2 font-medium text-red-800 dark:text-red-300">
              Refused: {s.reason ?? 'not safe to run'}
            </span>
          )}
        </li>
      ))}
    </ul>
  );
}

function describe(e: unknown): string {
  return describeError(e);
}
