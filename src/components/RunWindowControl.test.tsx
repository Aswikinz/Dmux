import { useState } from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { RunWindowControl, splitRunWindow } from './RunWindowControl';

function Harness({ initial, onValue }: { initial?: string; onValue?: (v: string | undefined) => void }) {
  const [value, setValue] = useState<string | undefined>(initial);
  return (
    <RunWindowControl
      value={value}
      onChange={(v) => {
        setValue(v);
        onValue?.(v);
      }}
    />
  );
}

describe('RunWindowControl', () => {
  it('is off by default and shows no time fields', () => {
    render(<Harness />);
    expect((screen.getByRole('checkbox') as HTMLInputElement).checked).toBe(false);
    expect(screen.queryByLabelText('Run from')).toBeNull();
  });

  it('turns on with a night window and reports it as HH:MM-HH:MM', () => {
    const seen: (string | undefined)[] = [];
    render(<Harness onValue={(v) => seen.push(v)} />);
    fireEvent.click(screen.getByRole('checkbox'));
    expect(seen.at(-1)).toBe('22:00-05:00');
    expect((screen.getByLabelText('Run from') as HTMLInputElement).value).toBe('22:00');
    fireEvent.change(screen.getByLabelText('Until'), { target: { value: '06:30' } });
    expect(seen.at(-1)).toBe('22:00-06:30');
    fireEvent.change(screen.getByLabelText('Run from'), { target: { value: '21:15' } });
    expect(seen.at(-1)).toBe('21:15-06:30');
  });

  it('turning it off clears the window and turning it on again keeps the times', () => {
    const seen: (string | undefined)[] = [];
    render(<Harness initial="20:00-04:00" onValue={(v) => seen.push(v)} />);
    fireEvent.click(screen.getByRole('checkbox'));
    expect(seen.at(-1)).toBeUndefined();
    fireEvent.click(screen.getByRole('checkbox'));
    // The harness started at 20:00-04:00, but the control only remembers the
    // times it was shown with while it was off: the defaults for a window that
    // began as a value are the value's own times.
    expect(seen.at(-1)).toBe('20:00-04:00');
  });

  it('warns when start and end are the same', () => {
    render(<Harness initial="22:00-22:00" />);
    expect(screen.getByRole('alert').textContent).toContain('must be different');
  });

  it('splits a window text and falls back to the defaults', () => {
    expect(splitRunWindow('9:05-17:00')).toEqual({ start: '09:05', end: '17:00' });
    expect(splitRunWindow(undefined)).toEqual({ start: '22:00', end: '05:00' });
    expect(splitRunWindow('nonsense')).toEqual({ start: '22:00', end: '05:00' });
  });
});
