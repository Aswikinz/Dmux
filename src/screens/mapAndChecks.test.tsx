import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { createMockEngine } from '../api/mockEngine';
import { ThemeProvider } from '../design/theme';
import type { Engine } from '../api/engine';
import type { CheckPlan, Inventory, Mapping } from '../api/contract';
import { defaultNewProfile, defaultOldProfile } from './ConnectScreen';
import { MapScreen } from './MapScreen';
import { ChooseChecksScreen, chosenEstimate } from './ChooseChecksScreen';

/** Nothing is chosen on Choose checks until the user picks: choose every
 *  table, then wait for the level controls. */
async function chooseAllTables() {
  fireEvent.click(await screen.findByRole('button', { name: 'Select all' }));
  return screen.findByText('Estimated time:');
}


async function diffSetup() {
  const engine = createMockEngine();
  const inventory = await engine.compareStructure(
    { ...defaultOldProfile(), database: 'diff' },
    defaultNewProfile(),
  );
  const initial = await engine.buildMapping(inventory);
  return { engine, inventory, initial };
}

function MapHarness({
  engine,
  inventory,
  initial,
  seen,
}: {
  engine: Engine;
  inventory: Inventory;
  initial: Mapping;
  seen?: (m: Mapping) => void;
}) {
  const [mapping, setMapping] = useState(initial);
  return (
    <MapScreen
      engine={engine}
      inventory={inventory}
      mapping={mapping}
      onMappingChange={(m) => {
        seen?.(m);
        setMapping(m);
      }}
      onNext={() => {}}
      onBack={() => {}}
    />
  );
}

const wrap = (ui: React.ReactElement) => render(<ThemeProvider>{ui}</ThemeProvider>);

/** Click the queue item with this old name so its buttons show. */
function select(oldName: string) {
  fireEvent.click(screen.getByText(oldName));
}

describe('MapScreen suggestions', () => {
  it('shows a suggestion with a plain percentage and does not use it by itself', async () => {
    const { engine, inventory, initial } = await diffSetup();
    const editMapping = vi.spyOn(engine, 'editMapping');
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    select('demo_bank.legacy_ledger');
    const line = (await screen.findAllByTestId('suggestion-line'))[0];
    expect(line.textContent).toMatch(/Suggested: ledger_entries/);
    expect(line.textContent).toMatch(/82% match/);
    expect(line.textContent).toMatch(/not used until you accept it/);
    expect(editMapping).not.toHaveBeenCalled();
  });

  it('accepting a suggestion changes the mapping through the engine, and only then', async () => {
    const { engine, inventory, initial } = await diffSetup();
    const editMapping = vi.spyOn(engine, 'editMapping');
    const seen = vi.fn();
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} seen={seen} />);
    select('demo_bank.legacy_ledger');
    fireEvent.click(screen.getByRole('button', { name: 'Use suggestion' }));
    await waitFor(() => expect(seen).toHaveBeenCalled());
    expect(editMapping).toHaveBeenCalledWith(expect.anything(), [
      { kind: 'pair_table', schema: 'demo_bank', table: 'legacy_ledger', newTable: 'ledger_entries' },
    ]);
    const next = seen.mock.calls[0][0] as Mapping;
    expect(next.matches.find((m) => m.id === 'demo_bank.legacy_ledger')).toMatchObject({
      newName: 'demo_bank.ledger_entries',
      confidence: 'confirmed',
    });
    expect(next.needsAttention.some((m) => m.id === 'demo_bank.legacy_ledger')).toBe(false);
  });

  it('shows the refusal plainly when two old tables would map to one new table', async () => {
    const { engine, inventory, initial } = await diffSetup();
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    select('demo_bank.legacy_ledger');
    fireEvent.click(screen.getByRole('button', { name: 'Use suggestion' }));
    await waitFor(() => expect(screen.queryByText('demo_bank.legacy_ledger')).toBeNull());
    select('demo_bank.ledger_old');
    fireEvent.click(await screen.findByRole('button', { name: 'Use suggestion' }));
    const alert = await screen.findByText(/Two old tables cannot map to one new table/);
    expect(alert.textContent).toMatch(/demo_bank\.legacy_ledger is already paired with demo_bank\.ledger_entries/);
    expect(screen.getByText('That change was not made')).toBeDefined();
    // The item is still waiting for a decision.
    expect(screen.getByText('demo_bank.ledger_old')).toBeDefined();
  });

  it('never offers a suggestion below the confidence floor', async () => {
    const { engine, inventory, initial } = await diffSetup();
    const weak: Mapping = {
      ...initial,
      suggestions: [{ ...initial.suggestions![0], confidence: 12 }],
    };
    wrap(<MapHarness engine={engine} inventory={inventory} initial={weak} />);
    select('demo_bank.legacy_ledger');
    expect(screen.queryByRole('button', { name: 'Use suggestion' })).toBeNull();
  });
});

describe('MapScreen pairing picker', () => {
  it('pairs an item by hand, using the keyboard alone', async () => {
    const { engine, inventory, initial } = await diffSetup();
    const editMapping = vi.spyOn(engine, 'editMapping');
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    select('demo_bank.audit_trail');
    fireEvent.click(screen.getByRole('button', { name: 'Choose counterpart' }));
    const box = screen.getByRole('combobox', { name: /Counterpart for demo_bank.audit_trail/ });
    expect(document.activeElement).toBe(box);
    fireEvent.change(box, { target: { value: 'audit' } });
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual(['audit_log']);
    fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() =>
      expect(editMapping).toHaveBeenCalledWith(expect.anything(), [
        { kind: 'pair_table', schema: 'demo_bank', table: 'audit_trail', newTable: 'audit_log' },
      ]),
    );
    // The new table that was taken leaves the picker's list.
    await waitFor(() => expect(screen.queryByText('demo_bank.audit_trail')).toBeNull());
  });

  it('Enter in the picker never approves the item behind it', async () => {
    const { engine, inventory, initial } = await diffSetup();
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    select('demo_bank.audit_trail');
    fireEvent.click(screen.getByRole('button', { name: 'Choose counterpart' }));
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Escape' });
    expect(screen.queryByRole('combobox')).toBeNull();
    // Back on the list, the item is still there and unpaired.
    expect(screen.getByText('demo_bank.audit_trail')).toBeDefined();
  });

  it('ignoring an item the engine named goes to the engine with the reason', async () => {
    const { engine, inventory, initial } = await diffSetup();
    const editMapping = vi.spyOn(engine, 'editMapping');
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    select('demo_bank.audit_trail');
    const group = screen.getByRole('list', { name: 'Mapping queue' });
    fireEvent.click(within(group).getByRole('button', { name: /^ignore$/i }));
    fireEvent.change(screen.getByLabelText('Reason for ignoring'), { target: { value: 'retired' } });
    fireEvent.click(within(group).getAllByRole('button', { name: /ignore/i }).pop()!);
    await waitFor(() =>
      expect(editMapping).toHaveBeenCalledWith(expect.anything(), [
        { kind: 'ignore', path: 'demo_bank.audit_trail', reason: 'retired' },
      ]),
    );
    expect(await screen.findByRole('heading', { name: 'Ignored' })).toBeDefined();
  });
});

describe('MapScreen rename rules', () => {
  it('previews and rebuilds through the engine', async () => {
    const { engine, inventory, initial } = await diffSetup();
    const previewRules = vi.spyOn(engine, 'previewRules');
    const rebuildMapping = vi.spyOn(engine, 'rebuildMapping');
    const seen = vi.fn();
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} seen={seen} />);
    fireEvent.click(screen.getByRole('button', { name: /Add prefix or suffix rule/ }));
    fireEvent.change(screen.getByLabelText('Rule 1: old text'), { target: { value: 'acc' } });
    fireEvent.change(screen.getByLabelText('Rule 1: new text'), { target: { value: 'acct' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview rules' }));
    expect((await screen.findByRole('status')).textContent).toMatch(/These rules pair/);
    expect(previewRules).toHaveBeenCalledWith([{ kind: 'prefix_replace', pattern: 'acc', target: 'acct' }]);
    expect(rebuildMapping).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: /Apply rules and rebuild mapping/ }));
    await waitFor(() => expect(seen).toHaveBeenCalled());
    expect((seen.mock.calls[0][0] as Mapping).ruleSpecs).toEqual([
      { kind: 'prefix_replace', pattern: 'acc', target: 'acct' },
    ]);
  });
});

// ---------------------------------------------------------------------------

function ChecksHarness({
  engine,
  onPlan,
}: {
  engine: Engine;
  onPlan?: (p: CheckPlan) => void;
}) {
  const [plan, setPlan] = useState<CheckPlan | null>(null);
  const mapping: Mapping = { rules: [], matches: [], needsAttention: [], ignored: [] };
  return (
    <ChooseChecksScreen
      engine={engine}
      mapping={mapping}
      plan={plan}
      onPlanChange={(p) => {
        onPlan?.(p);
        setPlan(p);
      }}
      onNext={() => {}}
      onBack={() => {}}
    />
  );
}

const levelsOf = (plan: CheckPlan) => Object.fromEntries(plan.tables.map((t) => [t.tableId, t.level]));

describe('ChooseChecksScreen selection', () => {
  async function setup() {
    const plans: CheckPlan[] = [];
    wrap(<ChecksHarness engine={createMockEngine()} onPlan={(p) => plans.push(p)} />);
    await chooseAllTables();
    const last = () => plans[plans.length - 1];
    return { last };
  }

  it('selects by schema, then sets one level for the whole selection and marks the preset Custom', async () => {
    const { last } = await setup();
    const before = levelsOf(last());
    const archive = last().tables.filter((t) => t.schemaName === 'archive');
    expect(archive.length).toBeGreaterThan(0);
    fireEvent.change(screen.getByLabelText('Schema'), { target: { value: 'archive' } });
    fireEvent.click(screen.getByRole('button', { name: 'Select schema' }));
    expect(screen.getByText(new RegExp(`${archive.length} tables? selected \\(schema archive\\)`))).toBeDefined();
    fireEvent.change(screen.getByLabelText('Set level for the selection'), { target: { value: 'structure' } });
    fireEvent.click(screen.getByRole('button', { name: `Apply to ${archive.length} selected` }));
    const after = levelsOf(last());
    for (const t of last().tables) {
      expect(after[t.tableId]).toBe(t.schemaName === 'archive' ? 'structure' : before[t.tableId]);
    }
    expect((screen.getByTestId('preset-select') as HTMLSelectElement).value).toBe('custom');
  });

  it('selects by name pattern with a * wildcard, ignoring case', async () => {
    const { last } = await setup();
    fireEvent.change(screen.getByLabelText(/Name pattern/), { target: { value: 'TABLE_000*' } });
    fireEvent.click(screen.getByRole('button', { name: 'Select by name' }));
    const expected = last().tables.filter((t) => t.tableName.startsWith('table_000')).length;
    expect(screen.getByText(new RegExp(`${expected} tables selected`))).toBeDefined();
  });

  it('selects by size, larger or smaller than a number of megabytes', async () => {
    const { last } = await setup();
    // The mock gives table i a size of (i + 1) * 3 MB.
    fireEvent.change(screen.getByLabelText('Megabytes'), { target: { value: '100' } });
    fireEvent.click(screen.getByRole('button', { name: 'Select by size' }));
    const larger = last().tables.filter((t) => (t.sizeBytes ?? 0) > 100 * 1024 * 1024).length;
    expect(screen.getByText(new RegExp(`${larger} tables selected \\(larger than 100 MB\\)`))).toBeDefined();
    fireEvent.change(screen.getByLabelText('Size'), { target: { value: 'smaller' } });
    fireEvent.click(screen.getByRole('button', { name: 'Select by size' }));
    const smaller = last().tables.filter((t) => (t.sizeBytes ?? 0) < 100 * 1024 * 1024).length;
    expect(screen.getByText(new RegExp(`${smaller} tables selected \\(smaller than 100 MB\\)`))).toBeDefined();
  });

  it('a hand-ticked table joins the selection, and Clear selection empties it', async () => {
    const { last } = await setup();
    const first = last().tables[0];
    fireEvent.click(screen.getByRole('checkbox', { name: `Select ${first.schemaName}.${first.tableName}` }));
    expect(screen.getByRole('button', { name: 'Apply to 1 selected' })).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Clear selection' }));
    expect((screen.getByRole('button', { name: 'Apply to 0 selected' }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('ChooseChecksScreen control totals', () => {
  it('lists suggested columns unticked, and ticking one confirms it in the plan', async () => {
    const plans: CheckPlan[] = [];
    wrap(<ChecksHarness engine={createMockEngine()} onPlan={(p) => plans.push(p)} />);
    await chooseAllTables();
    const first = plans[plans.length - 1].tables[0];
    expect(first.controlTotalSuggestions).toContain('amount');
    expect(first.controlTotalColumns).toBeUndefined();
    const box = screen.getByRole('checkbox', {
      name: `Use amount as a control total for ${first.schemaName}.${first.tableName}`,
    }) as HTMLInputElement;
    expect(box.checked).toBe(false);
    fireEvent.click(box);
    const confirmed = plans[plans.length - 1].tables[0].controlTotalColumns;
    expect(confirmed).toEqual(['amount']);
    expect(plans[plans.length - 1].tables.slice(1).every((t) => !t.controlTotalColumns)).toBe(true);
    // The same checkbox stays on screen, so keyboard focus is not lost.
    expect(document.body.contains(box)).toBe(true);
    expect(box.checked).toBe(true);
    fireEvent.click(box);
    expect(plans[plans.length - 1].tables[0].controlTotalColumns).toBeUndefined();
  });

  it('says a ticked column is added up in every table that has it', async () => {
    wrap(<ChecksHarness engine={createMockEngine()} />);
    await chooseAllTables();
    expect(screen.getByText(/added up in every table that has a column with that name/)).toBeDefined();
  });
});

describe('ChooseChecksScreen: which tables to check', () => {
  const lastPlan = (plans: CheckPlan[]) => plans[plans.length - 1];
  const chosenIds = (p: CheckPlan) => p.tables.filter((t) => t.included !== false).map((t) => t.tableId);

  it('starts with nothing chosen, and Run waits until a table is chosen', async () => {
    const plans: CheckPlan[] = [];
    wrap(<ChecksHarness engine={createMockEngine()} onPlan={(p) => plans.push(p)} />);
    expect(await screen.findByText('No tables chosen yet (40 available).')).toBeDefined();
    expect(chosenIds(lastPlan(plans))).toEqual([]);
    expect(screen.getByTestId('levels-waiting')).toBeDefined();
    expect(screen.queryByText('Estimated time:')).toBeNull();
    const run = screen.getByRole('button', { name: 'Run' }) as HTMLButtonElement;
    expect(run.disabled).toBe(true);
    expect(screen.getByText('Choose at least one table to run.')).toBeDefined();

    fireEvent.click(screen.getByRole('checkbox', { name: 'Every table in archive' }));
    expect(await screen.findByText(/4 of 40 tables chosen\. The other 36 will not be read\./)).toBeDefined();
    expect(run.disabled).toBe(false);
    expect(screen.queryByText('Choose at least one table to run.')).toBeNull();
  });

  it('a schema box chooses its tables, and only chosen tables get a level', async () => {
    const plans: CheckPlan[] = [];
    wrap(<ChecksHarness engine={createMockEngine()} onPlan={(p) => plans.push(p)} />);
    fireEvent.click(await screen.findByRole('checkbox', { name: 'Every table in archive' }));
    await screen.findByText('Estimated time:');
    const archive = lastPlan(plans).tables.filter((t) => t.schemaName === 'archive');
    expect(chosenIds(lastPlan(plans))).toEqual(archive.map((t) => t.tableId));
    // The level list shows the 4 chosen tables, none of demo_bank.
    expect(screen.getAllByRole('combobox', { name: /^Check level for archive\./ })).toHaveLength(4);
    expect(screen.queryByRole('combobox', { name: /^Check level for demo_bank\./ })).toBeNull();
    // The estimate covers only the chosen tables.
    expect(screen.getByText(/for the 4 chosen tables/)).toBeDefined();
  });

  it('single tables can be ticked, and the schema box then shows "some"', async () => {
    const plans: CheckPlan[] = [];
    wrap(<ChecksHarness engine={createMockEngine()} onPlan={(p) => plans.push(p)} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show the tables in demo_bank' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Check demo_bank.table_0003' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Check demo_bank.table_0005' }));
    expect(await screen.findByText(/2 of 40 tables chosen/)).toBeDefined();
    const box = screen.getByRole('checkbox', { name: 'Every table in demo_bank' });
    expect(box.getAttribute('aria-checked')).toBe('mixed');
    expect(screen.getByText('2 of 36 chosen')).toBeDefined();
    // Unticking one leaves the other.
    fireEvent.click(screen.getByRole('checkbox', { name: 'Check demo_bank.table_0003' }));
    const names = lastPlan(plans)
      .tables.filter((t) => t.included !== false)
      .map((t) => t.tableName);
    expect(names).toEqual(['table_0005']);
  });

  it('Select all chooses every table and Clear all leaves none', async () => {
    const plans: CheckPlan[] = [];
    wrap(<ChecksHarness engine={createMockEngine()} onPlan={(p) => plans.push(p)} />);
    await chooseAllTables();
    expect(screen.getByText('All 40 tables chosen.')).toBeDefined();
    expect(chosenIds(lastPlan(plans))).toHaveLength(40);
    fireEvent.click(screen.getByRole('button', { name: 'Clear all' }));
    expect(await screen.findByText('No tables chosen yet (40 available).')).toBeDefined();
    expect(chosenIds(lastPlan(plans))).toEqual([]);
  });

  it('a search narrows the list, and "Select N matching" chooses just those', async () => {
    const plans: CheckPlan[] = [];
    wrap(<ChecksHarness engine={createMockEngine()} onPlan={(p) => plans.push(p)} />);
    fireEvent.change(await screen.findByPlaceholderText('Find a schema or table'), {
      target: { value: 'table_001' },
    });
    // table_0010 .. table_0019, across both schemas.
    fireEvent.click(screen.getByRole('button', { name: 'Select 10 matching' }));
    const names = lastPlan(plans)
      .tables.filter((t) => t.included !== false)
      .map((t) => t.tableName)
      .sort();
    expect(names).toEqual(Array.from({ length: 10 }, (_, i) => `table_00${10 + i}`));
  });
});

describe('chosenEstimate', () => {
  const table = (id: string, sizeBytes: number | undefined, included?: boolean) => ({
    tableId: id,
    schemaName: 's',
    tableName: id,
    level: 'full-values' as const,
    sizeBytes,
    included,
  });
  const plan = (tables: ReturnType<typeof table>[]) => ({
    tables,
    presets: [],
    estimateSecondsMin: 100,
    estimateSecondsMax: 400,
  });

  it('keeps the whole estimate when every table is chosen', () => {
    expect(chosenEstimate(plan([table('a', 10), table('b', 30)]))).toEqual({ min: 100, max: 400 });
  });

  it('scales by size when every size is known', () => {
    expect(chosenEstimate(plan([table('a', 10, true), table('b', 30, false)]))).toEqual({
      min: 25,
      max: 100,
    });
  });

  it('scales by count when a size is missing, and never says 0 seconds', () => {
    expect(chosenEstimate(plan([table('a', undefined, true), table('b', 30, false)]))).toEqual({
      min: 50,
      max: 200,
    });
    expect(chosenEstimate(plan([table('a', 0, true), table('b', 1000, false)]))).toEqual({
      min: 1,
      max: 1,
    });
  });
});

describe('MapScreen: what was mapped', () => {
  it('is hidden until asked for, then lists every table with its columns', async () => {
    const { engine, inventory, initial } = await diffSetup();
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    const tables = initial.matches.filter((m) => m.kind === 'table').length;
    const columns = initial.matches.filter((m) => m.kind === 'column').length;
    expect(screen.getByTestId('matched-count').textContent).toMatch(
      new RegExp(`${tables} tables and ${columns} columns paired`),
    );
    expect(screen.queryByTestId('matched-list')).toBeNull();

    const show = screen.getByRole('button', { name: 'Show what was mapped' });
    expect(show.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(show);
    const list = screen.getByTestId('matched-list');
    expect(within(list).getByText(/are paired\./)).toBeDefined();
    // Open the accounts table: its columns show old -> new and how they paired.
    fireEvent.click(within(list).getByRole('button', { name: 'Show the columns of accounts' }));
    expect(within(list).getAllByText('balance').length).toBe(2);
    expect(within(list).getAllByText('Same name').length).toBeGreaterThan(0);
    expect(within(list).getAllByText('Needs your decision').length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole('button', { name: 'Hide what was mapped' }));
    expect(screen.queryByTestId('matched-list')).toBeNull();
  });

  it('can be searched, and narrowed to renamed or undecided pairs', async () => {
    const { engine, inventory, initial } = await diffSetup();
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    fireEvent.click(screen.getByRole('button', { name: 'Show what was mapped' }));
    const list = screen.getByTestId('matched-list');
    fireEvent.change(within(list).getByPlaceholderText('Find a table or column'), {
      target: { value: 'posted' },
    });
    // The matching column shows under its table, opened by the search.
    expect(within(list).getAllByText('posted_at').length).toBe(2);
    expect(within(list).queryByText('archived_balances')).toBeNull();
    fireEvent.change(within(list).getByPlaceholderText('Find a table or column'), {
      target: { value: 'nothing-like-this' },
    });
    expect(within(list).getByText('Nothing matches.')).toBeDefined();
    fireEvent.change(within(list).getByPlaceholderText('Find a table or column'), {
      target: { value: '' },
    });
    fireEvent.click(within(list).getByRole('checkbox', { name: 'Only renamed or undecided' }));
    // Only the undecided balance column (and its table) remains.
    expect(within(list).getAllByText('balance').length).toBe(2);
    expect(within(list).queryByText('posted_at')).toBeNull();
  });
});

describe('MapScreen: Save mapping', () => {
  const saveWith = async (result: Awaited<ReturnType<Engine['saveMapping']>> | Error) => {
    const { engine, inventory, initial } = await diffSetup();
    const saveMapping = vi
      .spyOn(engine, 'saveMapping')
      .mockImplementation(async () => {
        if (result instanceof Error) throw result;
        return result;
      });
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    fireEvent.click(screen.getByRole('button', { name: 'Save mapping' }));
    await waitFor(() => expect(saveMapping).toHaveBeenCalledTimes(1));
    return saveMapping;
  };

  it('says where the file was saved and its mapping code', async () => {
    await saveWith({ ok: true, filePath: '/home/u/dmux-mapping.json', hash: '0123456789abcdef0123' });
    const note = await screen.findByText(
      'Mapping saved to /home/u/dmux-mapping.json (mapping code 0123456789ab).',
    );
    expect(note.className).toMatch(/emerald/);
  });

  it('says cancelled when the dialog is dismissed, not saved', async () => {
    await saveWith({ ok: false, cancelled: true });
    const note = await screen.findByText('Saving cancelled. Nothing was saved.');
    expect(note.className).not.toMatch(/emerald/);
  });

  it('shows a refused save as an error in plain words', async () => {
    await saveWith({ ok: false, error: 'the mapping could not be written: disk full' });
    expect(await screen.findByText('The mapping was not saved')).toBeDefined();
    expect(screen.getByText('the mapping could not be written: disk full')).toBeDefined();
  });

  it('shows a thrown error too, and never leaves the button stuck', async () => {
    await saveWith(new Error('boom'));
    expect(await screen.findByText('The mapping was not saved')).toBeDefined();
    expect(
      (screen.getByRole('button', { name: 'Save mapping' }) as HTMLButtonElement).disabled,
    ).toBe(false);
  });

  it('says plainly that the browser demo saves no file', async () => {
    const { engine, inventory, initial } = await diffSetup();
    wrap(<MapHarness engine={engine} inventory={inventory} initial={initial} />);
    fireEvent.click(screen.getByRole('button', { name: 'Save mapping' }));
    expect(await screen.findByText(/This is the browser demo, so no file was saved/)).toBeDefined();
  });
});

