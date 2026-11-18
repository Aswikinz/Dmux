import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { useState } from 'react';
import type { ReactElement } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockEngine } from '../api/mockEngine';
import { createTauriEngine } from '../api/tauriEngine';
import { ThemeProvider } from '../design/theme';
import type { Engine } from '../api/engine';
import type { Results, RunDefinition, RunProgress } from '../api/contract';
import { defaultNewProfile, defaultOldProfile } from './ConnectScreen';
import { SettingsScreen } from './SettingsScreen';
import { RunScreen } from './RunScreen';
import { ResultsScreen } from './ResultsScreen';
import { ChooseChecksScreen } from './ChooseChecksScreen';
import { ReportScreen } from './ReportScreen';
import type { CheckPlan } from '../api/contract';

/** Nothing is chosen on Choose checks until the user picks: choose every
 *  table, then wait for the level controls. */
async function chooseAllTables() {
  fireEvent.click(await screen.findByRole('button', { name: 'Select all' }));
  return screen.findByText('Estimated time:');
}


const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const WARNING = 'A custom check runs with your full permissions. Only run scripts you trust.';

function wrap(ui: ReactElement) {
  return render(<ThemeProvider>{ui}</ThemeProvider>);
}

beforeEach(() => {
  invoke.mockReset();
});

describe('SettingsScreen', () => {
  it('starts with custom checks off and shows the warning in plain words', async () => {
    wrap(<SettingsScreen engine={createMockEngine()} />);
    const toggle = await screen.findByLabelText('Run custom checks after each comparison');
    expect((toggle as HTMLInputElement).checked).toBe(false);
    expect(screen.getByTestId('hook-warning').textContent).toContain(WARNING);
  });

  it('turns custom checks on and sends no path', async () => {
    const engine = createMockEngine();
    const update = vi.spyOn(engine, 'updateSettings');
    wrap(<SettingsScreen engine={engine} />);
    fireEvent.click(await screen.findByLabelText('Run custom checks after each comparison'));
    await waitFor(() => expect(update).toHaveBeenCalledWith({ customChecksEnabled: true }));
    expect(
      (screen.getByLabelText('Run custom checks after each comparison') as HTMLInputElement).checked,
    ).toBe(true);
  });

  it('lists scripts after choosing a folder and says why one is refused', async () => {
    wrap(<SettingsScreen engine={createMockEngine()} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Choose folder…' }));
    expect(await screen.findByText('balance_totals.py')).toBeDefined();
    expect(screen.getByText('Will run')).toBeDefined();
    expect(screen.getByText(/Refused: other users on this computer can change it/)).toBeDefined();
    expect(screen.getByTestId('hooks-folder').textContent).toBe('/home/you/custom-checks');
  });

  it('shows a refused folder as a plain problem', async () => {
    const engine = createMockEngine();
    const base = await engine.getSettings();
    engine.getSettings = async () => ({
      ...base,
      hooksFolder: '/shared',
      folderScan: { folderProblem: 'the folder is refused: it belongs to a different user', scripts: [] },
    });
    wrap(<SettingsScreen engine={engine} />);
    expect((await screen.findByRole('alert')).textContent).toContain('will not run anything from this folder');
  });

  it('says so when no Python is found, and lets the user choose one', async () => {
    const engine = createMockEngine();
    const base = await engine.getSettings();
    engine.getSettings = async () => ({ ...base, pythonFound: undefined });
    const choose = vi.spyOn(engine, 'choosePython');
    wrap(<SettingsScreen engine={engine} />);
    expect((await screen.findByTestId('python-found')).textContent).toContain('No Python was found');
    fireEvent.click(screen.getByRole('button', { name: 'Choose Python…' }));
    await waitFor(() => expect(choose).toHaveBeenCalled());
  });

  it('saves a valid time limit and refuses an invalid one', async () => {
    const engine = createMockEngine();
    const update = vi.spyOn(engine, 'updateSettings');
    wrap(<SettingsScreen engine={engine} />);
    const input = (await screen.findByLabelText(/Time limit/)) as HTMLInputElement;
    fireEvent.change(input, { target: { value: '0' } });
    fireEvent.blur(input);
    expect(update).not.toHaveBeenCalled();
    expect(screen.getByText('Enter a whole number from 1 to 600.')).toBeDefined();
    fireEvent.change(input, { target: { value: '90' } });
    fireEvent.blur(input);
    await waitFor(() => expect(update).toHaveBeenCalledWith({ timeoutSecs: 90 }));
  });

  it('holds the appearance and reduced-effects switches', async () => {
    wrap(<SettingsScreen engine={createMockEngine()} />);
    fireEvent.click(await screen.findByLabelText('Dark'));
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(screen.getByLabelText('Reduce visual effects')).toBeDefined();
  });

  it('shows a plain error when a setting cannot be changed', async () => {
    const engine = createMockEngine();
    engine.updateSettings = async () => {
      throw new Error('the time limit must be between 1 and 600 seconds');
    };
    wrap(<SettingsScreen engine={engine} />);
    fireEvent.click(await screen.findByLabelText('Run custom checks after each comparison'));
    expect(await screen.findByText(/must be between 1 and 600/)).toBeDefined();
  });
});

function withCustomChecks(base: Results): Results {
  return {
    ...base,
    verdict: 'FAIL',
    customChecks: {
      findings: [
        { id: 'balance-sum', severity: 'fail', message: 'totals differ by 12', table: 'demo_bank.accounts' },
        { id: 'note', severity: 'info', message: 'looks fine' },
      ],
      scripts: [{ name: 'balance.py', sha256: 'a'.repeat(64) }],
      warnings: ['slow.py: not run: it belongs to a different user'],
    },
  };
}

describe('Results: custom checks', () => {
  function Harness({ engine }: { engine: Engine }) {
    const [results, setResults] = useState<Results | null>(null);
    return (
      <ResultsScreen
        engine={engine}
        runId="run-pass"
        results={results}
        onResultsChange={setResults}
        onNext={() => {}}
        onBack={() => {}}
      />
    );
  }

  it('shows each finding with its severity as a word, and the folded FAIL verdict', async () => {
    const engine = createMockEngine({ forcedFixture: 'pass' });
    const pass = await engine.getResults('run-pass');
    engine.getResults = async () => withCustomChecks(pass);
    wrap(<Harness engine={engine} />);
    expect(await screen.findByRole('heading', { name: 'Custom checks' })).toBeDefined();
    expect(screen.getByText('Failed')).toBeDefined();
    expect(screen.getByText('Note')).toBeDefined();
    expect(screen.getByText('totals differ by 12')).toBeDefined();
    expect(screen.getByText(/not run: it belongs to a different user/)).toBeDefined();
    expect(screen.getAllByText('FAIL').length).toBeGreaterThan(0);
  });

  it('shows no custom checks section when none ran', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'pass' })} />);
    await screen.findByText(/A PASS means/);
    expect(screen.queryByRole('heading', { name: 'Custom checks' })).toBeNull();
  });
});

describe('Run: custom check warning before the run', () => {
  const definition: RunDefinition = {
    oldProfile: defaultOldProfile(),
    newProfile: defaultNewProfile(),
    mapping: { rules: [], matches: [], needsAttention: [], ignored: [] },
    plan: { tables: [], presets: [], estimateSecondsMin: 1, estimateSecondsMax: 2 },
  };
  function Harness({ engine }: { engine: Engine }) {
    const [progress, setProgress] = useState<RunProgress | null>(null);
    return (
      <RunScreen
        engine={engine}
        definition={definition}
        progress={progress}
        onProgressChange={setProgress}
        onNext={() => {}}
        onBack={() => {}}
      />
    );
  }

  it('waits for the user to accept the warning when custom checks are on', async () => {
    const engine = createMockEngine({ forcedFixture: 'pass' });
    await engine.updateSettings({ customChecksEnabled: true });
    const start = vi.spyOn(engine, 'startRun');
    wrap(<Harness engine={engine} />);
    expect((await screen.findByTestId('hook-warning')).textContent).toContain(WARNING);
    expect(start).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Start with custom checks' }));
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  });

  it('starts at once, with no warning, when custom checks are off', async () => {
    const engine = createMockEngine({ forcedFixture: 'pass' });
    const start = vi.spyOn(engine, 'startRun');
    wrap(<Harness engine={engine} />);
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('hook-warning')).toBeNull();
  });
});

describe('Tauri adapter: settings and custom checks', () => {
  const raw = {
    custom_checks_enabled: true,
    hooks_folder: '/home/u/checks',
    scan: {
      folder_problem: null,
      scripts: [
        { name: 'a.py', usable: true, reason: null },
        { name: 'b.py', usable: false, reason: 'it belongs to a different user' },
      ],
    },
    python_path: null,
    python_source: 'bundled',
    python_found: '/app/python/bin/python3',
    timeout_secs: 30,
    warning: WARNING,
  };

  it('maps the settings and sends no path in an update', async () => {
    invoke.mockResolvedValue(raw);
    const engine = createTauriEngine();
    const s = await engine.updateSettings({ customChecksEnabled: true, timeoutSecs: 30 });
    expect(invoke).toHaveBeenCalledWith('update_settings', {
      input: { custom_checks_enabled: true, timeout_secs: 30 },
    });
    expect(s.hooksFolder).toBe('/home/u/checks');
    expect(s.folderScan?.scripts[1]).toEqual({ name: 'b.py', usable: false, reason: 'it belongs to a different user' });
    expect(s.pythonSource).toBe('bundled');
    expect(s.pythonPath).toBeUndefined();
  });

  it('opens the dialogs through the backend commands, never from the webview', async () => {
    invoke.mockResolvedValue(raw);
    const engine = createTauriEngine();
    await engine.chooseHooksFolder();
    await engine.choosePython();
    await engine.clearHooksFolder();
    await engine.clearPython();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual([
      'choose_hooks_folder',
      'choose_python',
      'clear_hooks_folder',
      'clear_python',
    ]);
  });

  it('turns a backend error into readable text', async () => {
    invoke.mockRejectedValueOnce({ kind: 'invalid_input', message: 'that is not a folder' });
    await expect(createTauriEngine().chooseHooksFolder()).rejects.toThrow('that is not a folder');
  });

  it('maps custom checks and treats an unknown severity as a failure', async () => {
    invoke.mockResolvedValueOnce({
      run_id: 'r',
      verdict: 'FAIL',
      coverage: { total_tables: 0, full_values_tables: 0, counts_tables: 0, structure_tables: 0, ignored_tables: 0 },
      ignored: [],
      failures: [],
      table_results: [],
      options_applied: false,
      options_recorded: true,
      warnings: [],
      custom_checks: {
        findings: [
          { id: 'a', severity: 'fail', message: 'm', table: 't' },
          { id: 'b', severity: 'catastrophic', message: 'm2', table: null },
        ],
        scripts: [{ name: 's.py', sha256: 'abc' }],
        warnings: ['w'],
      },
    });
    const res = await createTauriEngine().getResults('r');
    expect(res.customChecks?.findings.map((f) => f.severity)).toEqual(['fail', 'fail']);
    expect(res.customChecks?.findings[1].table).toBeUndefined();
    expect(res.customChecks?.scripts[0].name).toBe('s.py');
  });
});

describe('Settings: run defaults, reports and about', () => {
  it('shows the defaults: 2 and 4 scans, suggested level, keys off', async () => {
    wrap(<SettingsScreen engine={createMockEngine()} />);
    expect(((await screen.findByLabelText(/old database/)) as HTMLInputElement).value).toBe('2');
    expect((screen.getByLabelText(/new database/) as HTMLInputElement).value).toBe('4');
    expect((screen.getByLabelText(/Starting check level/) as HTMLSelectElement).value).toBe('suggested');
    expect((screen.getByLabelText(/include row keys/) as HTMLInputElement).checked).toBe(false);
  });

  it('saves a starting level, valid scan limits and the keys default', async () => {
    const engine = createMockEngine();
    const update = vi.spyOn(engine, 'updateSettings');
    wrap(<SettingsScreen engine={engine} />);
    fireEvent.change(await screen.findByLabelText(/Starting check level/), { target: { value: 'counts' } });
    await waitFor(() => expect(update).toHaveBeenCalledWith({ startingLevel: 'counts' }));
    const old = screen.getByLabelText(/old database/);
    fireEvent.change(old, { target: { value: '3' } });
    fireEvent.blur(old);
    await waitFor(() => expect(update).toHaveBeenCalledWith({ oldParallelism: 3 }));
    fireEvent.click(screen.getByLabelText(/include row keys/));
    await waitFor(() => expect(update).toHaveBeenCalledWith({ includeKeysDefault: true }));
  });

  it('refuses a scan limit below 1 without calling the backend', async () => {
    const engine = createMockEngine();
    const update = vi.spyOn(engine, 'updateSettings');
    wrap(<SettingsScreen engine={engine} />);
    const field = await screen.findByLabelText(/new database/);
    fireEvent.change(field, { target: { value: '0' } });
    fireEvent.blur(field);
    expect(update).not.toHaveBeenCalled();
    expect(screen.getByText('Enter a whole number from 1 to 32.')).toBeDefined();
  });

  it('has no statement-timeout setting (the engine cannot honour one)', async () => {
    wrap(<SettingsScreen engine={createMockEngine()} />);
    await screen.findByText('Run defaults');
    expect(screen.queryByText(/statement/i)).toBeNull();
  });

  it('shows the version, the data folder and the licence, read-only', async () => {
    wrap(<SettingsScreen engine={createMockEngine()} />);
    expect((await screen.findByTestId('about-version')).textContent).toBe('0.0.1-mock');
    expect(screen.getByTestId('about-data-dir').textContent).toContain('verify.dmux.desktop');
    expect(screen.getByText(/open source under the Apache-2.0 licence/)).toBeDefined();
    expect(screen.getByTestId('about-data-dir').closest('dd')?.querySelector('input')).toBeNull();
  });

  it('Choose checks starts from the saved starting level', async () => {
    const engine = createMockEngine();
    await engine.updateSettings({ startingLevel: 'structure' });
    function Harness() {
      const [plan, setPlan] = useState<CheckPlan | null>(null);
      return (
        <ChooseChecksScreen
          engine={engine}
          mapping={{ rules: [], matches: [], needsAttention: [], ignored: [] }}
          plan={plan}
          onPlanChange={setPlan}
          onNext={() => {}}
          onBack={() => {}}
        />
      );
    }
    wrap(<Harness />);
    await chooseAllTables();
    await waitFor(() =>
      expect((screen.getByTestId('preset-select') as HTMLSelectElement).value).toBe('structure'),
    );
    const levels = screen.getAllByLabelText(/Check level for/) as HTMLSelectElement[];
    expect(levels.length).toBeGreaterThan(0);
    expect(levels.every((l) => l.value === 'structure')).toBe(true);
  });

  it('the Report screen starts the keys box from the saved default', async () => {
    const engine = createMockEngine();
    await engine.updateSettings({ includeKeysDefault: true });
    const results = await engine.getResults('run-pass');
    wrap(<ReportScreen engine={engine} results={results} onBack={() => {}} onRestart={() => {}} />);
    await waitFor(() =>
      expect((screen.getByLabelText(/row keys|keys/i) as HTMLInputElement).checked).toBe(true),
    );
  });

  it('the Report screen keeps keys off when the default is off', async () => {
    const engine = createMockEngine();
    const results = await engine.getResults('run-pass');
    wrap(<ReportScreen engine={engine} results={results} onBack={() => {}} onRestart={() => {}} />);
    expect((screen.getByLabelText(/row keys|keys/i) as HTMLInputElement).checked).toBe(false);
  });

  it('maps the new settings through the Tauri adapter', async () => {
    invoke.mockResolvedValue({
      custom_checks_enabled: false, hooks_folder: null, scan: null, python_path: null,
      python_source: 'auto', python_found: null, timeout_secs: 60, warning: WARNING,
      starting_level: 'full_values', old_parallelism: 3, new_parallelism: 5,
      include_keys_default: true, app_version: '1.2.3', data_dir: '/d', licence: 'Apache-2.0',
    });
    const engine = createTauriEngine();
    const s = await engine.updateSettings({ startingLevel: 'full-values', oldParallelism: 3 });
    expect(invoke).toHaveBeenCalledWith('update_settings', {
      input: {
        custom_checks_enabled: undefined, timeout_secs: undefined,
        starting_level: 'full_values', old_parallelism: 3,
        new_parallelism: undefined, include_keys_default: undefined,
      },
    });
    expect(s).toMatchObject({
      startingLevel: 'full-values', oldParallelism: 3, newParallelism: 5,
      includeKeysDefault: true, appVersion: '1.2.3', dataDir: '/d', licence: 'Apache-2.0',
    });
  });
});
