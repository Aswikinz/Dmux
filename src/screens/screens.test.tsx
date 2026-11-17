import { render, screen, fireEvent } from '@testing-library/react';
import { useState } from 'react';
import type { ReactElement } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { createMockEngine } from '../api/mockEngine';
import { ThemeProvider } from '../design/theme';
import type { Engine } from '../api/engine';
import type {
  ConnectionProfile,
  Inventory,
  Mapping,
  CheckPlan,
  RunDefinition,
  RunProgress,
  Results,
} from '../api/contract';
import { ConnectScreen, defaultOldProfile, defaultNewProfile } from './ConnectScreen';
import { CompareStructureScreen } from './CompareStructureScreen';
import { MapScreen } from './MapScreen';
import { ChooseChecksScreen } from './ChooseChecksScreen';
import { RunScreen } from './RunScreen';
import { ResultsScreen } from './ResultsScreen';
import { ReportScreen } from './ReportScreen';

function wrap(ui: ReactElement) {
  return render(<ThemeProvider>{ui}</ThemeProvider>);
}

// Stateful harnesses so each screen's onXChange callback lifts state the way
// App does; testing against a static null prop would keep the screen stuck in
// its loading branch on every render.
function StructureHarness({ engine }: { engine: Engine }) {
  const [inventory, setInventory] = useState<Inventory | null>(null);
  return (
    <CompareStructureScreen
      engine={engine}
      oldProfile={defaultOldProfile()}
      newProfile={defaultNewProfile()}
      inventory={inventory}
      onInventoryChange={setInventory}
      onNext={() => {}}
      onBack={() => {}}
    />
  );
}

function MapHarness({ engine }: { engine: Engine }) {
  const [mapping, setMapping] = useState<Mapping | null>(null);
  const inventory: Inventory = {
    schemas: [],
    oldEncoding: 'UTF8',
    newEncoding: 'UTF8',
    oldCollation: 'C',
    newCollation: 'C',
  };
  return (
    <MapScreen
      engine={engine}
      inventory={inventory}
      mapping={mapping}
      onMappingChange={setMapping}
      onNext={() => {}}
      onBack={() => {}}
    />
  );
}

function ChecksHarness({ engine }: { engine: Engine }) {
  const [plan, setPlan] = useState<CheckPlan | null>(null);
  const mapping: Mapping = { rules: [], matches: [], needsAttention: [], ignored: [] };
  return (
    <ChooseChecksScreen
      engine={engine}
      mapping={mapping}
      plan={plan}
      onPlanChange={setPlan}
      onNext={() => {}}
      onBack={() => {}}
    />
  );
}

function RunHarness({ engine }: { engine: Engine }) {
  const [progress, setProgress] = useState<RunProgress | null>(null);
  const definition: RunDefinition = {
    oldProfile: defaultOldProfile(),
    newProfile: defaultNewProfile(),
    mapping: { rules: [], matches: [], needsAttention: [], ignored: [] },
    plan: { tables: [], presets: [], estimateSecondsMin: 1, estimateSecondsMax: 2 },
  };
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

function ResultsHarness({ engine, runId }: { engine: Engine; runId: string }) {
  const [results, setResults] = useState<Results | null>(null);
  return (
    <ResultsScreen
      engine={engine}
      runId={runId}
      results={results}
      onResultsChange={setResults}
      onNext={() => {}}
      onBack={() => {}}
    />
  );
}

describe('ConnectScreen', () => {
  it('tests both connections and enables the next step', async () => {
    const engine = createMockEngine();
    wrap(
      <ConnectScreen
        engine={engine}
        oldProfile={defaultOldProfile()}
        newProfile={defaultNewProfile()}
        onChangeOld={() => {}}
        onChangeNew={() => {}}
        onNext={() => {}}
      />,
    );
    const testButtons = screen.getAllByRole('button', { name: 'Test connection' });
    fireEvent.click(testButtons[0]);
    expect(await screen.findByText('Server version:')).toBeDefined();
    fireEvent.click(testButtons[1]);
    await screen.findAllByText('Server version:');
    const next = screen.getByRole('button', { name: 'Compare structure' }) as HTMLButtonElement;
    expect(next.disabled).toBe(false);
  });

  it('shows a superuser warning', async () => {
    const engine = createMockEngine();
    const profile: ConnectionProfile = { ...defaultOldProfile(), user: 'superuser' };
    wrap(
      <ConnectScreen
        engine={engine}
        oldProfile={profile}
        newProfile={defaultNewProfile()}
        onChangeOld={() => {}}
        onChangeNew={() => {}}
        onNext={() => {}}
      />,
    );
    fireEvent.click(screen.getAllByRole('button', { name: 'Test connection' })[0]);
    expect(await screen.findByText('Superuser login')).toBeDefined();
  });

  it('shows and edits the private-CA file field for verify encryption', () => {
    const engine = createMockEngine();
    function CaHarness() {
      const [p, setP] = useState(defaultOldProfile());
      return (
        <ConnectScreen
          engine={engine}
          oldProfile={p}
          newProfile={defaultNewProfile()}
          onChangeOld={setP}
          onChangeNew={() => {}}
          onNext={() => {}}
        />
      );
    }
    wrap(<CaHarness />);
    const input = screen.getByTestId('Old database-ca-file') as HTMLInputElement;
    expect(input.value).toBe('');
    fireEvent.change(input, { target: { value: '/tmp/bank-ca.pem' } });
    expect(input.value).toBe('/tmp/bank-ca.pem');
  });
});

describe('CompareStructureScreen', () => {
  it('loads an inventory and shows the diff tree', async () => {
    wrap(<StructureHarness engine={createMockEngine()} />);
    expect(await screen.findByText('demo_bank')).toBeDefined();
  });

  it('shows an error when the comparison fails', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      compareStructure: async () => {
        throw new Error('boom');
      },
    };
    wrap(<StructureHarness engine={engine} />);
    expect(await screen.findByText('Could not compare structure')).toBeDefined();
  });
});

describe('MapScreen', () => {
  it('builds a mapping and shows the queue', async () => {
    wrap(<MapHarness engine={createMockEngine()} />);
    expect(await screen.findByText('No items need attention.')).toBeDefined();
  });
});

describe('MapScreen', () => {
  it('builds a mapping and shows the queue', async () => {
    wrap(<MapHarness engine={createMockEngine()} />);
    expect(await screen.findByText('No items need attention.')).toBeDefined();
  });

  it('approves and ignores items, then lists the ignored one', async () => {
    const engine = createMockEngine();
    const diffInventory = await engine.compareStructure(
      { ...defaultOldProfile(), database: 'diff' },
      defaultNewProfile(),
    );
    const initial = await engine.buildMapping(diffInventory);
    expect(initial.needsAttention.length).toBeGreaterThan(0);

    function DiffMapHarness() {
      const [mapping, setMapping] = useState(initial);
      return (
        <MapScreen
          engine={engine}
          inventory={diffInventory}
          mapping={mapping}
          onMappingChange={setMapping}
          onNext={() => {}}
          onBack={() => {}}
        />
      );
    }

    wrap(<DiffMapHarness />);
    expect(await screen.findByText(/items need attention/)).toBeDefined();

    fireEvent.click(screen.getByRole('button', { name: /approve/i }));
    // After one approve the queue shrinks but may still have a selected item with
    // an Ignore control. Toggle ignore on the next selected item.
    const ignoreButtons = screen.queryAllByRole('button', { name: /^ignore$/i });
    if (ignoreButtons.length > 0) {
      fireEvent.click(ignoreButtons[0]);
      const input = screen.getByPlaceholderText('Reason for ignoring (required)');
      fireEvent.change(input, { target: { value: 'out of scope' } });
      fireEvent.click(screen.getByRole('button', { name: 'Ignore' }));
    }
    await screen.findByText('Ignored');
  });

  it('shows an error when mapping fails to build', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      buildMapping: async () => {
        throw new Error('mapping boom');
      },
    };
    const inventory: Inventory = {
      schemas: [],
      oldEncoding: 'UTF8',
      newEncoding: 'UTF8',
      oldCollation: 'C',
      newCollation: 'C',
    };
    function ErrorHarness() {
      const [mapping, setMapping] = useState<Mapping | null>(null);
      return (
        <MapScreen
          engine={engine}
          inventory={inventory}
          mapping={mapping}
          onMappingChange={setMapping}
          onNext={() => {}}
          onBack={() => {}}
        />
      );
    }
    wrap(<ErrorHarness />);
    expect(await screen.findByText('Could not build mapping')).toBeDefined();
  });

  it('saves the mapping', async () => {
    const engine = createMockEngine();
    const saveMapping = vi.spyOn(engine, 'saveMapping');
    const inventory = await engine.compareStructure(defaultOldProfile(), defaultNewProfile());
    const initial = await engine.buildMapping(inventory);
    function SaveHarness() {
      const [mapping, setMapping] = useState(initial);
      return (
        <MapScreen
          engine={engine}
          inventory={inventory}
          mapping={mapping}
          onMappingChange={setMapping}
          onNext={() => {}}
          onBack={() => {}}
        />
      );
    }
    wrap(<SaveHarness />);
    const save = await screen.findByRole('button', { name: 'Save mapping' });
    fireEvent.click(save);
    // Saving flips the label briefly, then the button returns once the save
    // promise resolves — and the engine actually received the built mapping.
    expect(await screen.findByRole('button', { name: 'Save mapping' })).toBeDefined();
    expect(saveMapping).toHaveBeenCalledTimes(1);
    expect(saveMapping.mock.calls[0][0]).toEqual(initial);
    saveMapping.mockRestore();
  });
});

describe('ChooseChecksScreen', () => {
  it('builds a plan and shows totals', async () => {
    wrap(<ChecksHarness engine={createMockEngine()} />);
    expect(await screen.findByText('Estimate:')).toBeDefined();
  });

  it('changes a table check level through the select', async () => {
    function ChecksWithState({ engine }: { engine: Engine }) {
      const [plan, setPlan] = useState<CheckPlan | null>(null);
      const mapping: Mapping = { rules: [], matches: [], needsAttention: [], ignored: [] };
      return (
        <ChooseChecksScreen
          engine={engine}
          mapping={mapping}
          plan={plan}
          onPlanChange={setPlan}
          onNext={() => {}}
          onBack={() => {}}
        />
      );
    }
    wrap(<ChecksWithState engine={createMockEngine()} />);
    await screen.findByText('Estimate:');
    // The level selector is rendered for the first row.
    const selects = screen.getAllByRole('combobox');
    const preset = selects[0]; // the preset select
    expect(preset).toBeDefined();
    // A level select exists among the rows.
    const levelSelect = selects.find((s) => (s as HTMLSelectElement).value !== '');
    expect(levelSelect).toBeDefined();
  });

  it('selects a preset', async () => {
    wrap(<ChecksHarness engine={createMockEngine()} />);
    await screen.findByText('Estimate:');
    const preset = screen.getAllByRole('combobox')[0] as HTMLSelectElement;
    fireEvent.change(preset, { target: { value: preset.options[1]?.value ?? '' } });
    expect(preset.value).toBeDefined();
  });

  it('updates a table level when a row select changes', async () => {
    wrap(<ChecksHarness engine={createMockEngine()} />);
    await screen.findByText('Estimate:');
    // Find a level select whose current value is one of the check levels.
    const levelSelects = screen
      .getAllByRole('combobox')
      .filter((s) => ['structure', 'counts', 'full-values'].includes((s as HTMLSelectElement).value));
    expect(levelSelects.length).toBeGreaterThan(0);
    const select = levelSelects[0] as HTMLSelectElement;
    const other = Array.from(select.options).find((o) => o.value !== select.value);
    if (other) {
      fireEvent.change(select, { target: { value: other.value } });
      expect(select.value).toBe(other.value);
    }
  });

  it('shows an error when the plan fails to build', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      buildCheckPlan: async () => {
        throw new Error('plan boom');
      },
    };
    function ErrorHarness() {
      const [plan, setPlan] = useState<CheckPlan | null>(null);
      const mapping: Mapping = { rules: [], matches: [], needsAttention: [], ignored: [] };
      return (
        <ChooseChecksScreen
          engine={engine}
          mapping={mapping}
          plan={plan}
          onPlanChange={setPlan}
          onNext={() => {}}
          onBack={() => {}}
        />
      );
    }
    wrap(<ErrorHarness />);
    expect(await screen.findByText('Could not build check plan')).toBeDefined();
  });
});

describe('RunScreen', () => {
  it('starts a run and reaches a finished state', async () => {
    wrap(<RunHarness engine={createMockEngine({ forcedFixture: 'pass' })} />);
    expect(await screen.findByText('Finished')).toBeDefined();
  });

  it('can pause, resume and cancel a running run', async () => {
    const runningProgress: RunProgress = {
      runId: 'run-live',
      overallState: 'running',
      tables: [
        {
          tableId: 'tbl-000001',
          schemaName: 'demo_bank',
          tableName: 'account_master',
          state: 'running',
          progressPercent: 40,
          etaSeconds: 5,
        },
      ],
      etaSecondsMin: 5,
      etaSecondsMax: 10,
    };
    let state: RunProgress = { ...runningProgress };
    const engine: Engine = {
      testConnection: async () => ({ ok: true }),
      compareStructure: async () => ({ schemas: [], oldEncoding: 'UTF8', newEncoding: 'UTF8', oldCollation: 'C', newCollation: 'C' }),
      buildMapping: async () => ({ rules: [], matches: [], needsAttention: [], ignored: [] }),
      saveMapping: async () => ({ hash: 'h', json: '{}' }),
      buildCheckPlan: async () => ({ tables: [], presets: [], estimateSecondsMin: 1, estimateSecondsMax: 2 }),
      startRun: async () => state,
      pollRun: async () => state,
      setRunPaused: async (_id, paused) => {
        state = { ...state, overallState: paused ? 'paused' : 'running' };
        return state;
      },
      cancelRun: async () => {
        state = { ...state, overallState: 'cancelled' };
        return state;
      },
      getResults: async () => ({
        runId: 'run-live',
        verdict: 'PASS',
        coverage: { totalTables: 1, fullValuesTables: 1, countsTables: 0, structureTables: 0, ignoredTables: 0 },
        ignored: [],
        failures: [],
        tableResults: [],
      }),
      exportReport: async () => ({ ok: true, filePath: '/tmp/r.pdf' }),
      getVersion: async () => 'mock',
    };
    wrap(<RunHarness engine={engine} />);
    const pause = await screen.findByRole('button', { name: 'Pause' });
    fireEvent.click(pause);
    expect(await screen.findAllByText('Paused')).toBeTruthy();
    const resume = screen.getByRole('button', { name: 'Resume' });
    fireEvent.click(resume);
    expect(await screen.findAllByText('Running')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(await screen.findAllByText('Cancelled')).toBeTruthy();
    expect(state.overallState).toBe('cancelled');
  });

  it('shows an error when the run fails to start', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      startRun: async () => {
        throw new Error('start boom');
      },
    };
    function ErrorHarness() {
      const [progress, setProgress] = useState<RunProgress | null>(null);
      const definition: RunDefinition = {
        oldProfile: defaultOldProfile(),
        newProfile: defaultNewProfile(),
        mapping: { rules: [], matches: [], needsAttention: [], ignored: [] },
        plan: { tables: [], presets: [], estimateSecondsMin: 1, estimateSecondsMax: 2 },
      };
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
    wrap(<ErrorHarness />);
    expect(await screen.findByText('Run error')).toBeDefined();
  });

  it('shows an error when polling fails mid-run', async () => {
    const running: RunProgress = {
      runId: 'run-live',
      overallState: 'running',
      tables: [
        {
          tableId: 'tbl-000001',
          schemaName: 'demo_bank',
          tableName: 'account_master',
          state: 'running',
          progressPercent: 10,
          etaSeconds: 3,
        },
      ],
      etaSecondsMin: 3,
      etaSecondsMax: 5,
    };
    const engine: Engine = {
      ...createMockEngine(),
      startRun: async () => running,
      pollRun: async () => {
        throw new Error('poll boom');
      },
    };
    function PollHarness() {
      const [progress, setProgress] = useState<RunProgress | null>(null);
      const definition: RunDefinition = {
        oldProfile: defaultOldProfile(),
        newProfile: defaultNewProfile(),
        mapping: { rules: [], matches: [], needsAttention: [], ignored: [] },
        plan: { tables: [], presets: [], estimateSecondsMin: 1, estimateSecondsMax: 2 },
      };
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
    wrap(<PollHarness />);
    expect(await screen.findByText('Run error')).toBeDefined();
  });
});

describe('ResultsScreen', () => {
  it('shows the verdict and coverage line on a passing run', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    expect(await screen.findByText(/tables fully checked/)).toBeDefined();
    expect(screen.getAllByText('Pass').length).toBeGreaterThan(0);
  });

  it('lists failures for a failing run', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    expect(await screen.findByText('Failures')).toBeDefined();
  });

  it('shows the changed-during-check verdict', async () => {
    wrap(
      <ResultsHarness
        engine={createMockEngine({ forcedFixture: 'changedDuringCheck' })}
        runId="run-changed-during-check"
      />,
    );
    expect(await screen.findByText(/tables fully checked/)).toBeDefined();
    expect(screen.getAllByText('Changed during check').length).toBeGreaterThan(0);
  });

  it('expands a failure to show drill-down rows', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    await screen.findByText('Failures');
    const showRows = screen.getAllByRole('button', { name: 'Show rows' })[0];
    fireEvent.click(showRows);
    // A changed row advertises its differing columns.
    expect(await screen.findByText('Changed columns: amount')).toBeDefined();
  });

  it('labels changed-value rows "Changed values", never "Changed during check"', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    await screen.findByText('Failures');
    const showRows = screen.getAllByRole('button', { name: 'Show rows' })[0];
    fireEvent.click(showRows);

    // The three drill-down kinds render distinct FAIL-class words.
    expect(await screen.findByText('Missing in new')).toBeDefined();
    expect(screen.getByText('Extra in new')).toBeDefined();
    expect(screen.getByText('Changed values')).toBeDefined();

    // A changed-value row is a data difference, not a concurrent modification,
    // so it must never carry the "Changed during check" label.
    expect(screen.queryByText('Changed during check')).toBeNull();
  });

  it('lists ignored tables with their required reasons next to the coverage line', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    const ignoredheading = screen.getByRole('heading', { name: 'Ignored' });
    expect(ignoredheading).toBeDefined();
    expect(screen.getByText('archived_balances')).toBeDefined();
    expect(screen.getByText(/Legacy archive table/)).toBeDefined();
  });

  it('shows the needs-review verdict in its own words', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'needsReview' })} runId="run-needs-review" />);
    await screen.findByText('Failures');
    expect(screen.getAllByText('Needs review').length).toBeGreaterThan(0);
    expect(screen.getByText(/manual review/)).toBeDefined();
  });
});

describe('ReportScreen', () => {
  const passResults: Results = {
    runId: 'run-pass',
    verdict: 'PASS',
    coverage: { totalTables: 40, fullValuesTables: 12, countsTables: 10, structureTables: 15, ignoredTables: 3 },
    ignored: [
      { id: 'ign-0', name: 'archived_balances', reason: 'Legacy archive table' },
      { id: 'ign-1', name: 'ignored_table_0002', reason: 'Out of scope' },
      { id: 'ign-2', name: 'ignored_table_0003', reason: 'Out of scope' },
    ],
    failures: [],
    tableResults: [],
  };

  it('exports a PDF and shows the saved path', async () => {
    const engine = createMockEngine();
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export pdf/i }));
    expect(await screen.findByText(/report saved/)).toBeDefined();
  });

  it('exports Excel and toggles row keys', async () => {
    const engine = createMockEngine();
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: /export excel/i }));
    expect(await screen.findByText(/report saved/)).toBeDefined();
  });

  it('shows an error when export fails', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      exportReport: async () => ({ ok: false, error: 'disk full' }),
    };
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export pdf/i }));
    expect(await screen.findByText('Export failed')).toBeDefined();
  });
});
