import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { createMockEngine } from '../api/mockEngine';
import type { CheckPlan } from '../api/contract';
import type { Engine } from '../api/engine';
import { CheckChoicesFile } from './CheckChoicesFile';

const plan: CheckPlan = {
  tables: [
    {
      tableId: 'core.loans',
      schemaName: 'core',
      tableName: 'loans',
      level: 'full-values',
      columns: [
        { name: 'id', typeName: 'integer', key: true, number: true },
        { name: 'amount', typeName: 'numeric', key: false, number: true },
      ],
    },
    { tableId: 'core.fees', schemaName: 'core', tableName: 'fees', level: 'full-values', included: false },
  ],
  presets: [],
  estimateSecondsMin: 1,
  estimateSecondsMax: 2,
};

function engineWith(overrides: Partial<Engine>): Engine {
  return { ...createMockEngine(), ...overrides };
}

describe('CheckChoicesFile', () => {
  it('saves the chosen tables and shows where the file went', async () => {
    const save = vi.fn().mockResolvedValue({ status: 'saved', path: '/home/x/checks.json' });
    render(<CheckChoicesFile engine={engineWith({ saveCheckChoices: save })} plan={plan} onPlanChange={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Save these choices…' }));
    expect(await screen.findByText('Saved the choices to /home/x/checks.json.')).toBeDefined();
    expect(save).toHaveBeenCalledWith({
      runWindow: undefined,
      tables: [{ table: 'core.loans', level: 'full-values', leftOutColumns: [], controlTotals: [] }],
    });
  });

  it('opens a file, applies it and says what could not be used', async () => {
    const onPlanChange = vi.fn();
    const open = vi.fn().mockResolvedValue({
      status: 'opened',
      path: '/home/x/checks.json',
      choices: {
        tables: [
          { table: 'core.fees', level: 'counts', leftOutColumns: [], controlTotals: [] },
          { table: 'core.gone', level: 'counts', leftOutColumns: [], controlTotals: [] },
        ],
      },
    });
    render(<CheckChoicesFile engine={engineWith({ openCheckChoices: open })} plan={plan} onPlanChange={onPlanChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Open saved choices…' }));
    const msg = await screen.findByTestId('check-choices-message');
    expect(msg.textContent).toContain('Opened /home/x/checks.json.');
    expect(msg.textContent).toContain('Applied the saved choices to 1 table.');
    expect(msg.textContent).toContain('core.gone');
    const next = onPlanChange.mock.calls[0][0] as CheckPlan;
    expect(next.tables.map((t) => [t.tableId, t.included, t.level])).toEqual([
      ['core.loans', false, 'full-values'],
      ['core.fees', true, 'counts'],
    ]);
  });

  it('says plainly when the dialog was closed, or the file was refused', async () => {
    const onPlanChange = vi.fn();
    const open = vi
      .fn()
      .mockResolvedValueOnce({ status: 'cancelled' })
      .mockRejectedValueOnce({ kind: 'invalid_input', message: 'this is not a Dmux check choices file' });
    render(<CheckChoicesFile engine={engineWith({ openCheckChoices: open })} plan={plan} onPlanChange={onPlanChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Open saved choices…' }));
    expect(await screen.findByText('Nothing opened: the open window was closed.')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Open saved choices…' }));
    await waitFor(() =>
      expect(screen.getByRole('alert').textContent).toContain('this is not a Dmux check choices file'),
    );
    expect(onPlanChange).not.toHaveBeenCalled();
  });

  it('in the browser demo, says files cannot be saved instead of pretending', async () => {
    render(<CheckChoicesFile engine={createMockEngine()} plan={plan} onPlanChange={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Save these choices…' }));
    expect(await screen.findByText(/browser demo cannot save files/)).toBeDefined();
  });
});
