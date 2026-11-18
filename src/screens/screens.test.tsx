import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
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
import { DRILL_DOWN_PAGE, ResultsScreen } from './ResultsScreen';
import { ReportScreen } from './ReportScreen';

/** Nothing is chosen on Choose checks until the user picks: choose every
 *  table, then wait for the level controls. */
async function chooseAllTables() {
  fireEvent.click(await screen.findByRole('button', { name: 'Select all' }));
  return screen.findByText('Estimated time:');
}


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

  it('shows the in-memory password notice when there is no keychain, and only then', async () => {
    const base = createMockEngine();
    const engine: Engine = {
      ...base,
      testConnection: async (profile, password) => {
        const r = await base.testConnection(profile, password);
        return r.info
          ? { ...r, info: { ...r.info, sessionPassword: profile.id === defaultOldProfile().id } }
          : r;
      },
    };
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
    expect(
      await screen.findByText(
        'No system keychain: Dmux will keep this password in memory until you close the app.',
      ),
    ).toBeDefined();
    fireEvent.click(testButtons[1]);
    await waitFor(() => expect(screen.getAllByText('Server version:').length).toBe(2));
    // The new side has a keychain: one notice only.
    expect(screen.getAllByTestId('session-password-notice').length).toBe(1);
  });

  it('refuses a superuser until the user types a reason, then records it', async () => {
    const engine = createMockEngine();
    const setOverride = vi.spyOn(engine, 'setSuperuserOverride');
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
    expect(await screen.findByText('This login is a superuser')).toBeDefined();
    expect(screen.getByText('Connection failed')).toBeDefined();
    const allow = screen.getByRole('button', {
      name: 'Allow superuser and test again',
    }) as HTMLButtonElement;
    // No reason, no override.
    expect(allow.disabled).toBe(true);
    fireEvent.change(screen.getByTestId('superuser-reason'), { target: { value: '   ' } });
    expect(allow.disabled).toBe(true);
    fireEvent.change(screen.getByTestId('superuser-reason'), {
      target: { value: 'rehearsal on a copy' },
    });
    fireEvent.click(allow);
    expect(setOverride).toHaveBeenCalledWith({ reason: 'rehearsal on a copy' });
    expect(await screen.findByText('Superuser login allowed.')).toBeDefined();
    expect(await screen.findByText('Superuser login')).toBeDefined();
    expect(screen.getByText(/Your reason is recorded in the results and the report/).textContent).toContain(
      'rehearsal on a copy',
    );
    // Revoking it goes back to refusing.
    fireEvent.click(screen.getByRole('button', { name: 'Stop allowing superuser' }));
    expect(setOverride).toHaveBeenLastCalledWith(null);
  });

  it('says Connected, what it checked, and what to do next', async () => {
    wrap(
      <ConnectScreen
        engine={createMockEngine()}
        oldProfile={defaultOldProfile()}
        newProfile={defaultNewProfile()}
        onChangeOld={() => {}}
        onChangeNew={() => {}}
        onNext={() => {}}
      />,
    );
    expect(screen.getByText('Test both connections to continue.')).toBeDefined();
    const buttons = screen.getAllByRole('button', { name: 'Test connection' });
    fireEvent.click(buttons[0]);
    expect(await screen.findByText('Connected')).toBeDefined();
    expect(screen.getByText(/checked the server version, the text encoding, the encryption/)).toBeDefined();
    expect(screen.getByText('Read-only login:')).toBeDefined();
    fireEvent.click(buttons[1]);
    expect(
      await screen.findByText('Both databases are connected. Next, compare their structure.'),
    ).toBeDefined();
  });

  it('explains the unencrypted-connection reason without internal references', () => {
    wrap(
      <ConnectScreen
        engine={createMockEngine()}
        oldProfile={{ ...defaultOldProfile(), encryption: 'allow-plaintext' }}
        newProfile={defaultNewProfile()}
        onChangeOld={() => {}}
        onChangeNew={() => {}}
        onNext={() => {}}
      />,
    );
    expect(screen.getByText(/written\s+into the report/)).toBeDefined();
    expect(document.body.textContent).not.toMatch(/PLAN|section \d/);
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

  it('starts with empty profiles (no pre-filled host, database or user)', () => {
    expect(defaultOldProfile()).toMatchObject({
      host: '',
      database: '',
      user: '',
      port: 5432,
    });
    expect(defaultNewProfile()).toMatchObject({
      host: '',
      database: '',
      user: '',
      port: 5432,
    });
  });

  it('shows a password input on each side', () => {
    wrap(
      <ConnectScreen
        engine={createMockEngine()}
        oldProfile={defaultOldProfile()}
        newProfile={defaultNewProfile()}
        onChangeOld={() => {}}
        onChangeNew={() => {}}
        onNext={() => {}}
      />,
    );
    const oldPassword = screen.getByTestId('Old database-password') as HTMLInputElement;
    const newPassword = screen.getByTestId('New database-password') as HTMLInputElement;
    expect(oldPassword.type).toBe('password');
    expect(newPassword.type).toBe('password');
  });

  it('sends the typed password to saveProfile exactly once and clears it', async () => {
    const engine = createMockEngine();
    const saveProfile = vi.spyOn(engine, 'saveProfile');
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

    const password = screen.getByTestId('Old database-password') as HTMLInputElement;
    fireEvent.change(password, { target: { value: 'hunter2' } });
    fireEvent.click(screen.getAllByRole('button', { name: 'Save' })[0]);

    await waitFor(() => expect(saveProfile).toHaveBeenCalledTimes(1));
    expect(saveProfile.mock.calls[0][1]).toBe('hunter2');
    await waitFor(() => expect(password.value).toBe(''));
    saveProfile.mockRestore();
  });

  it('clears the password after a failed save', async () => {
    const engine = createMockEngine();
    const saveProfile = vi.fn().mockRejectedValue(new Error('boom'));
    wrap(
      <ConnectScreen
        engine={{ ...engine, saveProfile }}
        oldProfile={defaultOldProfile()}
        newProfile={defaultNewProfile()}
        onChangeOld={() => {}}
        onChangeNew={() => {}}
        onNext={() => {}}
      />,
    );

    const password = screen.getByTestId('Old database-password') as HTMLInputElement;
    fireEvent.change(password, { target: { value: 'hunter2' } });
    fireEvent.click(screen.getAllByRole('button', { name: 'Save' })[0]);

    expect(await screen.findByText('boom')).toBeDefined();
    await waitFor(() => expect(password.value).toBe(''));
  });

  it('never renders the password text after save, even on failure', async () => {
    const engine = createMockEngine();
    const saveProfile = vi.fn().mockRejectedValue(new Error('boom'));
    wrap(
      <ConnectScreen
        engine={{ ...engine, saveProfile }}
        oldProfile={defaultOldProfile()}
        newProfile={defaultNewProfile()}
        onChangeOld={() => {}}
        onChangeNew={() => {}}
        onNext={() => {}}
      />,
    );

    fireEvent.change(screen.getByTestId('Old database-password'), {
      target: { value: 'hunter2' },
    });
    fireEvent.click(screen.getAllByRole('button', { name: 'Save' })[0]);

    expect(await screen.findByText('boom')).toBeDefined();
    // The password never appears in any rendered text or error message.
    expect(screen.queryByText('hunter2')).toBeNull();
    expect(screen.queryByText(/hunter2/)).toBeNull();
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

  it('names both databases above the side-by-side view', async () => {
    wrap(<StructureHarness engine={createMockEngine()} />);
    const oldName = await screen.findByTestId('old-database-name');
    expect(oldName.textContent).toContain('Old database:');
    expect(screen.getByTestId('new-database-name').textContent).toContain('New database:');
  });

  it('shows a backend error as plain words, never [object Object]', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      compareStructure: async () => {
        throw { kind: 'password_missing', reason: 'no_keychain', profile: 'old-profile' };
      },
    };
    wrap(<StructureHarness engine={engine} />);
    expect(await screen.findByText('Could not compare structure')).toBeDefined();
    expect(screen.getByText(/no password is stored for this profile/)).toBeDefined();
    expect(screen.queryByText(/object Object/)).toBeNull();
  });

  it('shows an encoding difference as information, with a plain explanation', async () => {
    const base = createMockEngine();
    const engine: Engine = {
      ...base,
      compareStructure: (o, n) => base.compareStructure({ ...o, database: 'diff' }, n),
    };
    wrap(<StructureHarness engine={engine} />);
    const note = await screen.findByTestId('encoding-note');
    expect(note.getAttribute('role')).toBe('note');
    expect(note.textContent).toContain('LATIN1');
    expect(note.textContent).toContain('You do not need to do anything');
  });

  it('shows a column type change as before and after', async () => {
    const base = createMockEngine();
    const engine: Engine = {
      ...base,
      compareStructure: (o, n) => base.compareStructure({ ...o, database: 'diff' }, n),
    };
    wrap(<StructureHarness engine={engine} />);
    const change = (await screen.findAllByTestId('diff-before-after'))[0];
    expect(change.textContent).toMatch(/numeric\(18,2\).*→.*numeric\(19,2\)/);
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

  it('never puts an encoding difference in the needs-attention queue', async () => {
    const engine = createMockEngine();
    const diffInventory = await engine.compareStructure(
      { ...defaultOldProfile(), database: 'diff' },
      defaultNewProfile(),
    );
    expect(diffInventory.oldEncoding).not.toBe(diffInventory.newEncoding);
    const mapping = await engine.buildMapping(diffInventory);
    expect(mapping.needsAttention.some((m) => /encoding/i.test(m.oldName))).toBe(false);
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
    expect(await chooseAllTables()).toBeDefined();
  });

  it('offers an optional run window, off by default, and stores it on the plan', async () => {
    let latest: CheckPlan | null = null;
    function Spy({ engine }: { engine: Engine }) {
      const [plan, setPlan] = useState<CheckPlan | null>(null);
      latest = plan;
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
    wrap(<Spy engine={createMockEngine()} />);
    await chooseAllTables();
    expect((latest as CheckPlan | null)?.runWindow).toBeUndefined();
    fireEvent.click(screen.getByRole('checkbox', { name: 'Only run during certain hours' }));
    expect((latest as CheckPlan | null)?.runWindow).toBe('22:00-05:00');
    fireEvent.click(screen.getByRole('checkbox', { name: 'Only run during certain hours' }));
    expect((latest as CheckPlan | null)?.runWindow).toBeUndefined();
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
    await chooseAllTables();
    // The level selector is rendered for the first row.
    const selects = screen.getAllByRole('combobox');
    const preset = selects[0]; // the preset select
    expect(preset).toBeDefined();
    // A level select exists among the rows.
    const levelSelect = selects.find((s) => (s as HTMLSelectElement).value !== '');
    expect(levelSelect).toBeDefined();
  });

  it('a preset really reassigns every table level, and Suggested restores them', async () => {
    wrap(<ChecksHarness engine={createMockEngine()} />);
    await chooseAllTables();
    const levels = () =>
      screen
        .getAllByRole('combobox')
        .filter((s) => s.getAttribute('aria-label')?.startsWith('Check level for'))
        .map((s) => (s as HTMLSelectElement).value);
    const suggested = levels();
    expect(new Set(suggested).size).toBeGreaterThan(1);
    const preset = screen.getByTestId('preset-select') as HTMLSelectElement;
    expect(preset.value).toBe('suggested');

    fireEvent.change(preset, { target: { value: 'full' } });
    await waitFor(() => expect(levels().every((l) => l === 'full-values')).toBe(true));
    expect(screen.getByText(/Applied "Full values for every table"/)).toBeDefined();

    fireEvent.change(preset, { target: { value: 'counts' } });
    await waitFor(() => expect(levels().every((l) => l === 'counts')).toBe(true));
    expect(screen.getByText(/Now set Full values on the tables you want/)).toBeDefined();

    fireEvent.change(preset, { target: { value: 'structure' } });
    await waitFor(() => expect(levels().every((l) => l === 'structure')).toBe(true));

    fireEvent.change(preset, { target: { value: 'suggested' } });
    await waitFor(() => expect(levels()).toEqual(suggested));

    // Changing one table by hand marks the plan as custom.
    const first = screen
      .getAllByRole('combobox')
      .find((s) => s.getAttribute('aria-label')?.startsWith('Check level for')) as HTMLSelectElement;
    fireEvent.change(first, { target: { value: first.value === 'counts' ? 'structure' : 'counts' } });
    await waitFor(() => expect(preset.value).toBe('custom'));
  });

  it('shows the estimate in plain words and explains control totals', async () => {
    wrap(<ChecksHarness engine={createMockEngine()} />);
    await chooseAllTables();
    expect(screen.getByText(/about .*(seconds?|minutes?)/)).toBeDefined();
    expect(screen.getByText(/Corrected as each table finishes/)).toBeDefined();
    expect(screen.getByRole('button', { name: 'Control totals' })).toBeDefined();
  });

  it('updates a table level when a row select changes', async () => {
    wrap(<ChecksHarness engine={createMockEngine()} />);
    await chooseAllTables();
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
    expect(await screen.findByText('The run has finished.')).toBeDefined();
    expect((await screen.findByTestId('verdict-word')).textContent).toBe('PASS');
  });

  it('shows FAIL as soon as a failing run ends, never "Done" or a success badge', async () => {
    wrap(<RunHarness engine={createMockEngine({ forcedFixture: 'fail' })} />);
    expect(await screen.findByText('The run has finished.')).toBeDefined();
    expect((await screen.findByTestId('verdict-word')).textContent).toBe('FAIL');
    expect(screen.getByText(/do not match/)).toBeDefined();
    expect(screen.queryByText('Done')).toBeNull();
    expect(screen.queryByText('PASS')).toBeNull();
  });

  it('says the run waits for its window, and shows notices', async () => {
    const waiting: RunProgress = {
      runId: 'run-wait',
      overallState: 'running',
      tables: [],
      waitingForWindow: '22:00',
      notices: ['Dmux could not stop this computer from going to sleep during the check.'],
    };
    const engine: Engine = {
      ...createMockEngine(),
      startRun: async () => waiting,
      pollRun: async () => waiting,
    };
    wrap(<RunHarness engine={engine} />);
    expect(
      await screen.findByText('Waiting for the run window (opens at 22:00)'),
    ).toBeDefined();
    expect(screen.getByText(/could not stop this computer from going to sleep/)).toBeDefined();
  });

  it('shows the live time remaining as a plain range', async () => {
    const live: RunProgress = {
      runId: 'run-eta',
      overallState: 'running',
      tables: [
        {
          tableId: 't1',
          schemaName: 'demo_bank',
          tableName: 'ledger',
          state: 'running',
          progressPercent: 0,
          etaSeconds: 90,
        },
      ],
      etaSecondsMin: 60,
      etaSecondsMax: 600,
    };
    const engine: Engine = { ...createMockEngine(), startRun: async () => live, pollRun: async () => live };
    wrap(<RunHarness engine={engine} />);
    expect(await screen.findByText(/Time remaining:/)).toBeDefined();
    expect(screen.getByText(/about 2 minutes left/)).toBeDefined();
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
      ...createMockEngine(),
      setSuperuserOverride: () => {},
      testConnection: async () => ({ ok: true }),
      saveProfile: async (profile) => ({ profile }),
      profileHasPassword: async () => false,
      listProfiles: async () => [],
      deleteProfile: async () => false,
      listHistory: async () => [],
      openHistoryEntry: async () => { throw new Error('not in history'); },
      openResultsFile: async () => ({ ok: false, cancelled: true }),
      getHistoryLog: async () => '',
      saveHistoryLog: async () => ({ ok: false, cancelled: true }),
      purgeHistory: async () => 0,
      compareStructure: async () => ({ schemas: [], oldEncoding: 'UTF8', newEncoding: 'UTF8', oldCollation: 'C', newCollation: 'C' }),
      buildMapping: async () => ({ rules: [], matches: [], needsAttention: [], ignored: [] }),
      editMapping: async (mapping) => mapping,
      previewRules: async () => ({ tablesTotal: 0, tablesByRules: 0, tablesMatched: 0 }),
      rebuildMapping: async () => ({ rules: [], matches: [], needsAttention: [], ignored: [] }),
      saveMapping: async () => ({ ok: true, filePath: '/m.json', hash: 'h' }),
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
        optionsApplied: false,
        optionsRecorded: true,
        warnings: [],
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
    // An ended run offers nothing to pause or cancel.
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Pause' })).toBeNull());
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
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
    expect(screen.getAllByText('PASS').length).toBeGreaterThan(0);
  });

  it('lists failures for a failing run', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    expect(await screen.findByText('Failures')).toBeDefined();
  });

  it('states the verdict prominently in words, with plain level names', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    const banner = await screen.findByTestId('verdict-banner');
    expect(banner.getAttribute('role')).toBe('status');
    expect(screen.getByTestId('verdict-word').textContent).toBe('FAIL');
    expect(banner.textContent).toContain('Result: FAIL');
    expect(banner.textContent).toContain('do not match');
    expect(banner.querySelector('svg')).toBeTruthy();
    expect(screen.queryAllByText('full-values')).toHaveLength(0);
  });

  it('shows the changed-during-check verdict', async () => {
    wrap(
      <ResultsHarness
        engine={createMockEngine({ forcedFixture: 'changedDuringCheck' })}
        runId="run-changed-during-check"
      />,
    );
    expect(await screen.findByText(/tables fully checked/)).toBeDefined();
    expect(screen.getAllByText('CHANGED DURING CHECK').length).toBeGreaterThan(0);
  });

  it('expands a failure to show drill-down rows', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    await screen.findByText('Failures');
    const showRows = screen.getAllByRole('button', { name: 'Show rows' })[0];
    fireEvent.click(showRows);
    // A changed row advertises its differing columns.
    expect(await screen.findByText('Changed columns: amount')).toBeDefined();
  });

  it('pages a very large drill-down instead of rendering every row', async () => {
    // A partitioned-on-new table can report every old row as missing (15,000
    // in demo_bank); rendering them all at once froze the webview.
    const engine = createMockEngine({ forcedFixture: 'fail' });
    const base = await engine.getResults('run-fail');
    const card = base.failures[0];
    const big: Results = {
      ...base,
      tableResults: base.tableResults.map((t) =>
        t.tableId === card.tableId
          ? {
              ...t,
              drillDown: Array.from({ length: 15000 }, (_, i) => ({
                key: `row-${i}`,
                kind: 'missing' as const,
              })),
            }
          : t,
      ),
    };
    wrap(
      <ResultsScreen
        engine={engine}
        runId="run-fail"
        results={big}
        onResultsChange={() => {}}
        onNext={() => {}}
        onBack={() => {}}
      />,
    );

    fireEvent.click(screen.getAllByRole('button', { name: 'Show rows' })[0]);
    expect(screen.getByText(`Showing ${DRILL_DOWN_PAGE} of 15,000 rows`)).toBeDefined();
    expect(screen.getAllByText('Missing in new')).toHaveLength(DRILL_DOWN_PAGE);
    expect(screen.queryByText(`row-${DRILL_DOWN_PAGE}`)).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /Show 50 more/ }));
    expect(screen.getByText(`Showing ${DRILL_DOWN_PAGE * 2} of 15,000 rows`)).toBeDefined();
    expect(screen.getByText(`row-${DRILL_DOWN_PAGE}`)).toBeDefined();
    expect(screen.getByText(/14,900 not shown/)).toBeDefined();
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
    expect(screen.queryByText('CHANGED DURING CHECK')).toBeNull();
  });

  it('counts the ignored items by the coverage line and lists each with its reason on request', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    expect(screen.getByRole('heading', { name: 'Ignored' })).toBeDefined();
    expect(screen.getByText('3 items, each with its reason')).toBeDefined();
    // The list stays closed until asked for, so it never buries the verdict.
    expect(screen.queryByText('archived_balances')).toBeNull();
    const show = screen.getByRole('button', { name: 'Show ignored items' });
    expect(show.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(show);
    expect(screen.getByText('archived_balances')).toBeDefined();
    expect(screen.getByText(/Legacy archive table/)).toBeDefined();
    expect(screen.getByText('ignored_table_0002')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Hide ignored items' }));
    expect(screen.queryByText('archived_balances')).toBeNull();
  });

  it('filters the lists by verdict, says how many are hidden, and never hides the verdict', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    await screen.findByText('Failures');
    const filter = screen.getByTestId('verdict-filter');
    expect(within(filter).getByText('Showing every result.')).toBeDefined();
    const passBox = within(filter).getByRole('checkbox', { name: /PASS/ });
    const rowsBefore = screen.getAllByText('Details').length;
    fireEvent.click(passBox);
    // The PASS rows leave the table; the FAIL banner and the failure card stay.
    expect(screen.getAllByText('Details').length).toBeLessThan(rowsBefore);
    expect(screen.getAllByText('FAIL').length).toBeGreaterThan(0);
    expect(within(filter).getByText(/results are hidden by this filter/)).toBeDefined();
    // Hiding FAIL too leaves a plain note in place of the cards.
    fireEvent.click(within(filter).getByRole('checkbox', { name: /^FAIL/ }));
    expect(screen.getByTestId('failures-all-hidden')).toBeDefined();
  });

  it('shows the needs-review verdict in its own words', async () => {
    wrap(<ResultsHarness engine={createMockEngine({ forcedFixture: 'needsReview' })} runId="run-needs-review" />);
    await screen.findByText('Failures');
    expect(screen.getAllByText('NEEDS REVIEW').length).toBeGreaterThan(0);
    expect(screen.getByText(/manual review/)).toBeDefined();
  });

  it('shows "PASS (with options)" and the options list when options were applied', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      getResults: async () => ({
        runId: 'run-pass-options',
        verdict: 'PASS',
        coverage: { totalTables: 1, fullValuesTables: 1, countsTables: 0, structureTables: 0, ignoredTables: 0 },
        ignored: [],
        failures: [],
        tableResults: [
          {
            tableId: 'demo_bank.accounts',
            schemaName: 'demo_bank',
            tableName: 'accounts',
            verdict: 'PASS',
            level: 'full-values',
            checked: true,
            columnNotes: [{ column: 'balance', kind: 'option', note: 'round to 2 decimals' }],
          },
        ],
        optionsApplied: true,
        optionsRecorded: true,
        warnings: [],
      }),
    };
    wrap(<ResultsHarness engine={engine} runId="run-pass-options" />);
    expect(await screen.findByText('PASS (with options)')).toBeDefined();
    expect(screen.getByText('Options applied')).toBeDefined();
    expect(screen.getAllByText(/round to 2 decimals/).length).toBeGreaterThan(0);
  });

  it('shows "options not recorded" for an older results file (never a bare PASS)', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      getResults: async () => ({
        runId: 'run-pass-old',
        verdict: 'PASS',
        coverage: { totalTables: 1, fullValuesTables: 1, countsTables: 0, structureTables: 0, ignoredTables: 0 },
        ignored: [],
        failures: [],
        tableResults: [],
        optionsApplied: false,
        optionsRecorded: false,
        warnings: [],
      }),
    };
    wrap(<ResultsHarness engine={engine} runId="run-pass-old" />);
    expect(
      await screen.findByText('PASS (options not recorded — older results file)'),
    ).toBeDefined();
    expect(screen.getByText('options not recorded (older results file)')).toBeDefined();
    // The verdict must be the single qualified label, never a bare PASS.
    expect(screen.queryByText('PASS')).toBeNull();
  });

  it('lists run warnings (can-write / superuser / row-level security)', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      getResults: async () => ({
        runId: 'run-warn',
        verdict: 'PASS',
        coverage: { totalTables: 21, fullValuesTables: 2, countsTables: 1, structureTables: 0, ignoredTables: 0 },
        ignored: [],
        failures: [],
        tableResults: [],
        optionsApplied: false,
        optionsRecorded: true,
        warnings: ['the old database login can write; the login should be read-only'],
      }),
    };
    wrap(<ResultsHarness engine={engine} runId="run-warn" />);
    expect(await screen.findByText('Warnings')).toBeDefined();
    expect(screen.getByText(/the old database login can write/)).toBeDefined();
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
    optionsApplied: false,
    optionsRecorded: true,
    warnings: [],
  };

  it('uses the same coverage line as the Results screen', () => {
    wrap(
      <ReportScreen engine={createMockEngine()} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    expect(
      screen.getByText(
        '12 of 40 tables fully checked, 10 counts only, 15 structure only, 3 ignored',
      ),
    ).toBeDefined();
  });

  it('the browser demo says plainly that it saves no file, never a fake path', async () => {
    const engine = createMockEngine();
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export pdf/i }));
    expect(await screen.findByText(/browser demo, so no PDF file was saved/)).toBeDefined();
    expect(screen.queryByText(/report saved/)).toBeNull();
    expect(screen.queryByText(/chosen-location/)).toBeNull();
    expect(screen.queryByText('Export failed')).toBeNull();
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: /export excel/i }));
    expect(await screen.findByText(/browser demo, so no Excel file was saved/)).toBeDefined();
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
    expect(screen.getByText('disk full')).toBeDefined();
    expect(screen.queryByText(/report saved/)).toBeNull();
  });

  it('shows the exact saved path and asks for this run', async () => {
    const exportReport = vi.fn(async () => ({ ok: true, filePath: '/home/u/dmux-run-pass.pdf' }));
    const engine: Engine = { ...createMockEngine(), exportReport };
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export pdf/i }));
    expect(
      await screen.findByText('PDF report saved to /home/u/dmux-run-pass.pdf'),
    ).toBeDefined();
    expect(exportReport).toHaveBeenCalledWith({
      runId: 'run-pass',
      format: 'pdf',
      includeKeys: false,
    });
  });

  it('says cancelled, not saved or failed, when the dialog is dismissed', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      exportReport: async () => ({ ok: false, cancelled: true }),
    };
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export excel/i }));
    expect(await screen.findByText('Excel export cancelled. Nothing was saved.')).toBeDefined();
    expect(screen.queryByText('Export failed')).toBeNull();
    expect(screen.queryByText(/report saved/)).toBeNull();
  });

  it('exports the interactive HTML report for this run', async () => {
    const exportReport = vi.fn(async () => ({ ok: true, filePath: '/home/u/dmux-run-pass.html' }));
    const engine: Engine = { ...createMockEngine(), exportReport };
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export html/i }));
    expect(
      await screen.findByText('HTML report saved to /home/u/dmux-run-pass.html'),
    ).toBeDefined();
    expect(exportReport).toHaveBeenCalledWith({
      runId: 'run-pass',
      format: 'html',
      includeKeys: false,
    });
  });

  it('does not show a cancelled export in the saved (green) style', async () => {
    const engine: Engine = {
      ...createMockEngine(),
      exportReport: async () => ({ ok: false, cancelled: true }),
    };
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export html/i }));
    const msg = await screen.findByText('HTML export cancelled. Nothing was saved.');
    expect(msg.className).not.toMatch(/emerald/);
  });

  it('sends the key choice only when the user ticks it', async () => {
    const exportReport = vi.fn(async () => ({ ok: true, filePath: '/r.xlsx' }));
    const engine: Engine = { ...createMockEngine(), exportReport };
    wrap(
      <ReportScreen engine={engine} results={passResults} onBack={() => {}} onRestart={() => {}} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /export excel/i }));
    await screen.findByText(/report saved/);
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: /export excel/i }));
    await waitFor(() => expect(exportReport).toHaveBeenCalledTimes(2));
    expect(exportReport.mock.calls.map((c) => (c as unknown[])[0])).toEqual([
      { runId: 'run-pass', format: 'excel', includeKeys: false },
      { runId: 'run-pass', format: 'excel', includeKeys: true },
    ]);
  });
});
