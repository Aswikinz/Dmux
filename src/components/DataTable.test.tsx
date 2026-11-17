import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { DataTable } from './DataTable';
import type { ColumnDef } from '@tanstack/react-table';

interface Row {
  id: string;
  name: string;
}

const data: Row[] = [
  { id: '1', name: 'alpha' },
  { id: '2', name: 'beta' },
  { id: '3', name: 'gamma' },
];

const columns: ColumnDef<Row>[] = [
  { header: 'Name', accessorKey: 'name' },
];

describe('DataTable', () => {
  it('renders rows and headers', () => {
    render(<DataTable data={data} columns={columns} />);
    expect(screen.getByText('Name')).toBeDefined();
    expect(screen.getByText('alpha')).toBeDefined();
    expect(screen.getByText('beta')).toBeDefined();
    expect(screen.getByText('gamma')).toBeDefined();
  });

  it('filters rows by text', () => {
    render(<DataTable data={data} columns={columns} />);
    fireEvent.change(screen.getByPlaceholderText('Search...'), { target: { value: 'beta' } });
    expect(screen.getByText('beta')).toBeDefined();
    expect(screen.queryByText('alpha')).toBeNull();
  });

  it('applies a custom filter function', () => {
    const filterFn = (row: Row, query: string) => row.id === query;
    render(<DataTable data={data} columns={columns} filterFn={filterFn} />);
    fireEvent.change(screen.getByPlaceholderText('Search...'), { target: { value: '2' } });
    expect(screen.getByText('beta')).toBeDefined();
    expect(screen.queryByText('alpha')).toBeNull();
  });

  
  it('does not re-filter, and so does not reset the table, when it re-renders with the same inputs', () => {
    // A fresh filtered array on every render made react-table reset forever once
    // a parent state change (such as opening a row's details) re-rendered it.
    const filterFn = vi.fn((row: Row, query: string) => row.name.includes(query));
    const { rerender } = render(<DataTable data={data} columns={columns} filterFn={filterFn} />);
    const calls = filterFn.mock.calls.length;
    rerender(<DataTable data={data} columns={columns} filterFn={filterFn} />);
    expect(filterFn.mock.calls.length).toBe(calls);
  });
});
