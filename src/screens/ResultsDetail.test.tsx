import { fireEvent, render, screen, within } from '@testing-library/react';
import { useState, type ReactElement } from 'react';
import { describe, expect, it } from 'vitest';
import { ThemeProvider } from '../design/theme';
import { createMockEngine } from '../api/mockEngine';
import type { Engine } from '../api/engine';
import type { Results, TableResult } from '../api/contract';
import { ResultsScreen } from './ResultsScreen';

function wrap(ui: ReactElement) {
  return render(<ThemeProvider>{ui}</ThemeProvider>);
}

function Harness({ engine, runId }: { engine: Engine; runId: string }) {
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

const open = (name: string) => fireEvent.click(screen.getByRole('button', { name }));
const panel = () => within(screen.getByTestId('table-detail'));

describe('Results detail for a table that passed', () => {
  it('opens from the All tables grid and explains why the table passed', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    open('Details for demo_bank.table_0000');

    const heading = screen.getByRole('heading', { name: 'Details for demo_bank.table_0000' });
    expect(document.activeElement).toBe(heading);
    const why = screen.getByTestId('why-this-table');
    expect(why.textContent).toContain('Why this table passed');
    expect(why.textContent).toContain('Both databases hold 1,000 rows');
    expect(why.textContent).toContain('all 256 buckets of rows matched');
    expect(why.textContent).toContain('spot check compared 200 rows value by value, and it agreed');
  });

  it('lists what was tested: level, rows, key, buckets, spot check, columns, totals, change detection', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    open('Details for demo_bank.table_0000');

    expect(screen.getByText(/Dmux compared every value in every compared column/)).toBeDefined();
    expect(screen.getByText(/Rows were matched by/).textContent).toContain('id');
    expect(screen.getByText(/Rows are summarised in/).textContent).toContain('256');
    expect(screen.getByText(/rows were fetched from both databases and compared value by value/)).toBeDefined();
    const cols = screen.getByRole('region', { name: 'Columns compared and the rule for each' });
    expect(within(cols).getByText('compared by its exact 8-byte binary form')).toBeDefined();
    expect(within(cols).getByText('compared exactly as text')).toBeDefined();
    const totals = screen.getByRole('region', { name: 'Control totals on both sides' });
    expect(within(totals).getAllByText('1482930.50')).toHaveLength(2);
    expect(within(totals).getByText('Match')).toBeDefined();
    expect(screen.getByText(/were the same before and after the table was read/)).toBeDefined();
  });

  it('shows the type change, an ignored column and its reason', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    open('Details for demo_bank.table_0007');
    expect(panel().getByText(/type changed from integer to text/)).toBeDefined();
    expect(panel().getByText(/free text, not migrated/)).toBeDefined();
  });

  it('closes again', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    open('Details for demo_bank.table_0000');
    fireEvent.click(screen.getByRole('button', { name: 'Close details' }));
    expect(screen.queryByTestId('why-this-table')).toBeNull();
  });

  it('says what a structure-only table did and did not test', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    open('Details for demo_bank.table_0001');
    expect(screen.getByText(/It did not read any rows/)).toBeDefined();
    expect(screen.getByTestId('why-this-table').textContent).toContain('No row was read');
    expect(screen.queryByText('Control totals')).toBeNull();
  });

  it('never prints a row value, only counts, column names and sums', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    await screen.findByText('Failures');
    fireEvent.click(screen.getByRole('button', { name: 'Show details for demo_bank.table_0001' }));
    // The mock drill-down rows carry TX-1001 / 120.00; the panel must not.
    const text = screen.getByTestId('table-detail').textContent ?? '';
    expect(text).not.toContain('TX-1001');
    expect(text).not.toContain('120.00');
  });
});

describe('Results detail for a table that failed', () => {
  it('opens from a failure card and explains why it failed', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'fail' })} runId="run-fail" />);
    await screen.findByText('Failures');
    fireEvent.click(screen.getByRole('button', { name: 'Show details for demo_bank.table_0001' }));
    const why = screen.getByTestId('why-this-table').textContent ?? '';
    expect(why).toContain('Why this table failed');
    expect(why).toContain('The old database has 1,037 rows and the new one has 1,025 rows');
    expect(why).toContain('12 rows missing in the new database');
    const totals = screen.getByRole('region', { name: 'Control totals on both sides' });
    expect(within(totals).getByText('Differ')).toBeDefined();
  });
});

describe('Results detail when change detection was unavailable or facts are absent', () => {
  async function withTable(patch: Partial<TableResult>) {
    const engine = createMockEngine({ forcedFixture: 'pass' });
    const base = await engine.getResults('run-pass');
    const results: Results = {
      ...base,
      tableResults: base.tableResults.map((t) => (t.tableId === 'tbl-000000' ? { ...t, ...patch } : t)),
    };
    wrap(
      <ResultsScreen
        engine={engine}
        runId="run-pass"
        results={results}
        onResultsChange={() => {}}
        onNext={() => {}}
        onBack={() => {}}
      />,
    );
    open('Details for demo_bank.table_0000');
  }

  it('explains an unavailable change detection and what the user can do', async () => {
    const engine = createMockEngine({ forcedFixture: 'pass' });
    const base = await engine.getResults('run-pass');
    const facts = base.tableResults[0].facts;
    await withTable({
      verdict: 'NEEDS_REVIEW',
      facts: facts ? { ...facts, changeDetectionOld: 'track_counts_off' } : undefined,
    });
    expect(screen.getByText(/track_counts/).textContent).toContain('needs review instead of passing');
  });

  it('is honest when the engine recorded no facts for the table', async () => {
    await withTable({ facts: undefined });
    expect(screen.getByText(/did not record the detailed facts/)).toBeDefined();
  });
});

describe('What was checked', () => {
  it('shows the connections, versions and digests of the run', async () => {
    wrap(<Harness engine={createMockEngine({ forcedFixture: 'pass' })} runId="run-pass" />);
    await screen.findByText(/tables fully checked/);
    expect(screen.getByRole('heading', { name: 'What was checked' })).toBeDefined();
    expect(screen.getAllByText(/Encrypted, and the server certificate was verified/)).toHaveLength(2);
    expect(screen.getAllByText(/The login is read-only/)).toHaveLength(2);
    expect(screen.getAllByText('Not a superuser login.')).toHaveLength(2);
    expect(screen.getByText('1.0.0 (mock)')).toBeDefined();
    expect(screen.getByText(/^9b1de4c0/)).toBeDefined();
    expect(screen.getByTestId('coverage-explained').textContent).toContain('compared value by value');
  });

  it('records a superuser override reason and a plaintext connection', async () => {
    const engine = createMockEngine({ forcedFixture: 'pass' });
    const base = await engine.getResults('run-pass');
    const facts = base.runFacts;
    if (!facts) throw new Error('fixture has no run facts');
    const results: Results = {
      ...base,
      runFacts: {
        ...facts,
        oldConnection: {
          ...facts.oldConnection,
          superuser: true,
          superuserReason: 'migration rehearsal approved by the DBA',
          tls: 'plaintext',
          plaintextReason: 'isolated lab network',
        },
      },
    };
    wrap(
      <ResultsScreen
        engine={engine}
        runId="run-pass"
        results={results}
        onResultsChange={() => {}}
        onNext={() => {}}
        onBack={() => {}}
      />,
    );
    expect(screen.getByText(/migration rehearsal approved by the DBA/)).toBeDefined();
    expect(screen.getByText(/isolated lab network/)).toBeDefined();
  });

  it('says plainly when the run recorded no connection facts', async () => {
    const engine = createMockEngine({ forcedFixture: 'pass' });
    const base = await engine.getResults('run-pass');
    wrap(
      <ResultsScreen
        engine={engine}
        runId="run-pass"
        results={{ ...base, runFacts: undefined }}
        onResultsChange={() => {}}
        onNext={() => {}}
        onBack={() => {}}
      />,
    );
    expect(screen.getByText(/was not recorded for this run/)).toBeDefined();
  });
});

/** The mock FAIL results plus the cards a real edge-case run produces: a
 *  finding inside a table (a dropped column), a database-wide finding, and a
 *  table that failed with no rows to show. */
async function edgeEngine(): Promise<Engine> {
  const base = createMockEngine({ forcedFixture: 'fail' });
  const real = await base.getResults('run-fail');
  const tableWithRows = real.tableResults.find((t) => (t.drillDown?.length ?? 0) > 0)!;
  const dropped: TableResult = {
    ...tableWithRows,
    tableId: 'structure.dropped_column',
    schemaName: 'structure',
    tableName: 'dropped_column',
    drillDown: undefined,
  };
  const results: Results = {
    ...real,
    tableResults: [...real.tableResults, dropped],
    failures: [
      ...real.failures,
      {
        id: 'struct-structure.dropped_column.legacy_flag',
        tableId: 'structure.dropped_column.legacy_flag',
        schemaName: 'structure',
        tableName: 'dropped_column.legacy_flag',
        level: 'structure',
        verdict: 'FAIL',
        summary: 'column missing in new: legacy_flag',
        detail: 'an inventory object differs between the two databases',
      },
      {
        id: 'struct-database.encoding',
        tableId: 'database.encoding',
        schemaName: 'database',
        tableName: 'encoding',
        level: 'structure',
        verdict: 'WARNING',
        summary: 'encoding differs',
        detail: 'an inventory object differs between the two databases',
      },
    ],
  };
  return { ...base, getResults: async () => results };
}

function cardOf(summary: string): HTMLElement {
  return screen.getByText(summary).closest('div.rounded-lg') as HTMLElement;
}

describe('Failure cards: Show rows and Show details always do something', () => {
  it('opens the details of the table a column finding belongs to, right under that card', async () => {
    wrap(<Harness engine={await edgeEngine()} runId="run-edge" />);
    await screen.findByText('Failures');
    const card = cardOf('column missing in new: legacy_flag');
    fireEvent.click(within(card).getByRole('button', { name: /^Show details for structure\.dropped_column/ }));
    const heading = screen.getByRole('heading', { name: 'Details for structure.dropped_column' });
    // Directly below the card that was clicked, not at the end of the list.
    expect(card.parentElement?.contains(heading)).toBe(true);
    expect(document.activeElement).toBe(heading);
    // The same button closes it again.
    fireEvent.click(within(card).getByRole('button', { name: /^Hide details/ }));
    expect(screen.queryByRole('heading', { name: 'Details for structure.dropped_column' })).toBeNull();
  });

  it('offers no buttons on a database-wide finding, which belongs to no table', async () => {
    wrap(<Harness engine={await edgeEngine()} runId="run-edge" />);
    await screen.findByText('Failures');
    expect(within(cardOf('encoding differs')).queryByRole('button')).toBeNull();
  });

  it('offers Show rows only where there are rows, and it shows them', async () => {
    wrap(<Harness engine={await edgeEngine()} runId="run-edge" />);
    await screen.findByText('Failures');
    expect(
      within(cardOf('column missing in new: legacy_flag')).queryByRole('button', { name: 'Show rows' }),
    ).toBeNull();
    const rows = screen.getAllByRole('button', { name: 'Show rows' });
    expect(rows.length).toBeGreaterThan(0);
    fireEvent.click(rows[0]);
    expect(screen.getByRole('button', { name: 'Hide rows' }).getAttribute('aria-expanded')).toBe('true');
    expect(screen.getAllByText(/missing in new|changed|extra in new/i).length).toBeGreaterThan(0);
  });
});

