import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { PairPicker } from './PairPicker';

const options = [
  { value: 'ledger_entries', label: 'ledger_entries' },
  { value: 'audit_log', label: 'audit_log' },
  { value: 'customer_notes', label: 'customer_notes' },
];

describe('PairPicker', () => {
  it('is a labelled search box over a list of options, with focus in the box', () => {
    render(<PairPicker label="Counterpart for legacy" options={options} onPick={() => {}} onCancel={() => {}} />);
    const box = screen.getByRole('combobox', { name: 'Counterpart for legacy' });
    expect(document.activeElement).toBe(box);
    expect(screen.getAllByRole('option')).toHaveLength(3);
  });

  it('narrows the list as you type, ignoring case', () => {
    render(<PairPicker label="Pick" options={options} onPick={() => {}} onCancel={() => {}} />);
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'LEDG' } });
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual(['ledger_entries']);
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'zzz' } });
    expect(screen.queryAllByRole('option')).toHaveLength(0);
    expect(screen.getByRole('status').textContent).toMatch(/No match/);
  });

  it('works with the keyboard alone: arrows move, Enter chooses, Escape closes', () => {
    const onPick = vi.fn();
    const onCancel = vi.fn();
    render(<PairPicker label="Pick" options={options} onPick={onPick} onCancel={onCancel} />);
    const box = screen.getByRole('combobox');
    expect(box.getAttribute('aria-activedescendant')).toBe(screen.getAllByRole('option')[0].id);
    fireEvent.keyDown(box, { key: 'ArrowDown' });
    fireEvent.keyDown(box, { key: 'ArrowDown' });
    fireEvent.keyDown(box, { key: 'ArrowUp' });
    expect(screen.getAllByRole('option')[1].getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(box, { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith('audit_log');
    fireEvent.keyDown(box, { key: 'Escape' });
    expect(onCancel).toHaveBeenCalled();
  });

  it('works with the mouse: click an option, or Cancel', () => {
    const onPick = vi.fn();
    const onCancel = vi.fn();
    render(<PairPicker label="Pick" options={options} onPick={onPick} onCancel={onCancel} />);
    fireEvent.click(screen.getByRole('option', { name: 'customer_notes' }));
    expect(onPick).toHaveBeenCalledWith('customer_notes');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalled();
  });

  it('says so when there is nothing left to pair with', () => {
    render(<PairPicker label="Pick" options={[]} onPick={() => {}} onCancel={() => {}} />);
    expect(screen.getByRole('status').textContent).toMatch(/Nothing in the new database is left/);
  });
});
