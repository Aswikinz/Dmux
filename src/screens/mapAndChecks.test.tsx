import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { createMockEngine } from '../api/mockEngine';
import { ThemeProvider } from '../design/theme';
import type { Engine } from '../api/engine';
import type { CheckPlan, Inventory, Mapping } from '../api/contract';
import { defaultNewProfile, defaultOldProfile } from './ConnectScreen';
import { MapScreen } from './MapScreen';
import { ChooseChecksScreen } from './ChooseChecksScreen';

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
    await screen.findByText('Estimated time:');
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
    await screen.findByText('Estimated time:');
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
    await screen.findByText('Estimated time:');
    expect(screen.getByText(/added up in every table that has a column with that name/)).toBeDefined();
  });
});
