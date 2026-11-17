import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { MappingQueue } from './MappingQueue';
import type { MappingMatch } from '../api/contract';

const queue: MappingMatch[] = [
  { id: 'm-1', oldName: 'accounts.balance', newName: 'accounts.balance', confidence: 'fuzzy', approved: false },
  { id: 'm-2', oldName: 'acct', newName: 'account_master', confidence: 'fuzzy', approved: false },
];

describe('MappingQueue', () => {
  it('shows the empty state when there is nothing to do', () => {
    render(<MappingQueue queue={[]} onApprove={() => {}} onIgnore={() => {}} />);
    expect(screen.getByText('No items need attention.')).toBeDefined();
  });

  it('lists items and approves the selected one', async () => {
    const onApprove = vi.fn();
    render(<MappingQueue queue={queue} onApprove={onApprove} onIgnore={() => {}} />);
    // First item selected by default; the old and new names both render.
    expect(screen.getAllByText('accounts.balance').length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('button', { name: /approve/i }));
    expect(onApprove).toHaveBeenCalledWith('m-1');
  });

  it('ignores the selected item with a required reason', async () => {
    const onIgnore = vi.fn();
    render(<MappingQueue queue={queue} onApprove={() => {}} onIgnore={onIgnore} />);
    fireEvent.click(screen.getByRole('button', { name: /^ignore$/i }));
    const input = screen.getByPlaceholderText('Reason for ignoring (required)');
    fireEvent.change(input, { target: { value: 'not in scope' } });
    fireEvent.click(screen.getByRole('button', { name: /ignore/i }));
    expect(onIgnore).toHaveBeenCalledWith('m-1', 'not in scope');
  });

  it('moves the selection and approves with the keyboard', () => {
    const onApprove = vi.fn();
    render(<MappingQueue queue={queue} onApprove={onApprove} onIgnore={() => {}} />);
    // ArrowDown moves from index 0 to 1.
    fireEvent.keyDown(window, { key: 'ArrowDown' });
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(onApprove).toHaveBeenCalledWith('m-2');
  });

  it('ignores the selected item with the keyboard', () => {
    const onIgnore = vi.fn();
    render(<MappingQueue queue={queue} onApprove={() => {}} onIgnore={onIgnore} />);
    fireEvent.keyDown(window, { key: 'Delete' });
    const input = screen.getByPlaceholderText('Reason for ignoring (required)');
    fireEvent.change(input, { target: { value: 'skip' } });
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(onIgnore).toHaveBeenCalledWith('m-1', 'skip');
  });
});
