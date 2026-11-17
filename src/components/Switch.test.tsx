import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Switch } from './Switch';

describe('Switch', () => {
  it('renders its label and reflects the checked state', () => {
    render(<Switch label="Show only problems" checked onChange={() => {}} />);
    const checkbox = screen.getByRole('checkbox') as HTMLInputElement;
    expect(checkbox.checked).toBe(true);
    expect(screen.getByText('Show only problems')).toBeDefined();
  });

  it('calls onChange with the new value when toggled', async () => {
    const onChange = vi.fn();
    render(<Switch label="Include keys" checked={false} onChange={onChange} />);
    fireEvent.click(screen.getByRole('checkbox'));
    expect(onChange).toHaveBeenCalledWith(true);
  });
});
