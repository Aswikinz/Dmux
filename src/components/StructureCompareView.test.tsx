import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { StructureRow } from '../api/contract';
import { StructureCompareView } from './StructureCompareView';

const rows: StructureRow[] = [
  { id: 's', depth: 0, kind: 'schema', name: 'bank', status: 'same', old: 'present', new: 'present', expandable: true },
  { id: 's/a', depth: 1, kind: 'table', name: 'accounts', status: 'same', old: '3 columns', new: '3 columns', expandable: true },
  { id: 's/a/id', depth: 2, kind: 'column', name: 'id', status: 'same', old: 'bigint', new: 'bigint', expandable: false },
  { id: 's/a/bal', depth: 2, kind: 'column', name: 'balance', status: 'changed', old: 'numeric(18,2)', new: 'numeric(19,2)', expandable: false },
  { id: 's/a/old', depth: 2, kind: 'column', name: 'legacy_flag', status: 'missing', old: 'boolean', expandable: false },
  { id: 's/a/new', depth: 2, kind: 'column', name: 'memo', status: 'added', new: 'text', expandable: false },
  { id: 's/b', depth: 1, kind: 'table', name: 'branches', status: 'same', old: '1 columns', new: '1 columns', expandable: true },
  { id: 's/b/c', depth: 2, kind: 'column', name: 'code', status: 'same', old: 'text', new: 'text', expandable: false },
];

const labels = {
  old: { title: 'Old database', detail: 'legacy · PostgreSQL 9.6 · LATIN1' },
  new: { title: 'New database', detail: 'target · PostgreSQL 17.2 · UTF8' },
};

function show(onlyProblems = false) {
  return render(<StructureCompareView rows={rows} showOnlyProblems={onlyProblems} {...labels} />);
}

const rowFor = (name: string) =>
  screen.getAllByTestId('structure-row').find((r) => within(r).queryByText(name, { selector: 'span' }));

describe('StructureCompareView', () => {
  it('labels both sides with their server version and encoding', () => {
    show();
    expect(screen.getByTestId('old-header').textContent).toContain('PostgreSQL 9.6 · LATIN1');
    expect(screen.getByTestId('new-header').textContent).toContain('PostgreSQL 17.2 · UTF8');
  });

  it('puts the old value on the left and the new value on the right for a changed item', () => {
    show();
    const r = rowFor('balance');
    expect(r).toBeDefined();
    const cells = within(r as HTMLElement).getAllByRole('gridcell');
    expect(cells[0].textContent).toBe('numeric(18,2)');
    expect(cells[0].getAttribute('data-side')).toBe('old');
    expect(cells[2].textContent).toBe('numeric(19,2)');
    expect(cells[2].getAttribute('data-side')).toBe('new');
    expect(within(r as HTMLElement).getByText('Changed')).toBeDefined();
  });

  it('marks a missing item with a gap on the right and the word Missing', () => {
    show();
    const r = rowFor('legacy_flag') as HTMLElement;
    const gap = r.querySelector('[data-side="new"]') as HTMLElement;
    expect(gap.getAttribute('data-gap')).toBe('true');
    expect(gap.textContent).toBe('Not in the new database');
    expect(within(r).getByText('Missing')).toBeDefined();
  });

  it('marks an added item with a gap on the left and the word Added', () => {
    show();
    const r = rowFor('memo') as HTMLElement;
    const gap = r.querySelector('[data-side="old"]') as HTMLElement;
    expect(gap.getAttribute('data-gap')).toBe('true');
    expect(gap.textContent).toBe('Not in the old database');
    expect(within(r).getByText('Added')).toBeDefined();
  });

  it('shows an identical item plainly, with no status badge', () => {
    show();
    const r = rowFor('id') as HTMLElement;
    expect(within(r).queryByText('Same')).toBeNull();
    expect(r.getAttribute('data-status')).toBe('same');
  });

  it('keeps a before-and-after reading for assistive technology', () => {
    show();
    expect(screen.getByTestId('diff-before-after').textContent).toMatch(/numeric\(18,2\).*→.*numeric\(19,2\)/);
  });

  it('shows only problems when asked', () => {
    show(true);
    expect(rowFor('id')).toBeUndefined();
    expect(rowFor('balance')).toBeDefined();
    expect(rowFor('branches')).toBeUndefined();
  });

  it('searches across tables and columns', () => {
    show();
    fireEvent.change(screen.getByLabelText('Search the structure'), { target: { value: 'branches' } });
    expect(rowFor('branches')).toBeDefined();
    expect(rowFor('accounts')).toBeUndefined();
  });

  it('says so when nothing matches', () => {
    show();
    fireEvent.change(screen.getByLabelText('Search the structure'), { target: { value: 'zzz' } });
    expect(screen.getByText(/Nothing matches/)).toBeDefined();
  });

  it('counts the differences and the rows shown', () => {
    show();
    expect(screen.getByTestId('structure-count').textContent).toMatch(/3 differences · showing 7 of 8 items/);
  });

  it('opens and closes a group with the mouse', () => {
    show();
    expect(rowFor('code')).toBeUndefined();
    fireEvent.click(screen.getByRole('button', { name: 'Expand branches' }));
    expect(rowFor('code')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Collapse branches' }));
    expect(rowFor('code')).toBeUndefined();
  });

  it('is operable from the keyboard', () => {
    show();
    const grid = screen.getByRole('treegrid');
    fireEvent.focus(grid);
    expect(grid.getAttribute('aria-activedescendant')).toMatch(/-row-0$/);
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    expect(grid.getAttribute('aria-activedescendant')).toMatch(/-row-1$/);
    fireEvent.keyDown(grid, { key: 'End' });
    expect(grid.getAttribute('aria-activedescendant')).toMatch(/-row-6$/);
    // The last row is the closed "branches" group: Right opens it, Left closes it.
    fireEvent.keyDown(grid, { key: 'ArrowRight' });
    expect(rowFor('code')).toBeDefined();
    fireEvent.keyDown(grid, { key: 'ArrowLeft' });
    expect(rowFor('code')).toBeUndefined();
    fireEvent.keyDown(grid, { key: 'Enter' });
    expect(rowFor('code')).toBeDefined();
    fireEvent.keyDown(grid, { key: 'Home' });
    expect(grid.getAttribute('aria-activedescendant')).toMatch(/-row-0$/);
  });
});
