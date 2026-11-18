import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import { VerdictFilter } from './VerdictFilter';
import { resultCounts } from '../screens/ResultsScreen';
import type { Results, Verdict } from '../api/contract';

function Harness({ counts }: { counts: Partial<Record<Verdict, number>> }) {
  const [hidden, setHidden] = useState<Set<Verdict>>(new Set());
  return (
    <>
      <VerdictFilter counts={counts} hidden={hidden} onChange={setHidden} />
      <output data-testid="hidden">{[...hidden].sort().join(',')}</output>
    </>
  );
}

describe('VerdictFilter', () => {
  it('shows a box per verdict present, gravest first, with its count', () => {
    render(<Harness counts={{ PASS: 30, WARNING: 12, FAIL: 3 }} />);
    const boxes = screen.getAllByRole('checkbox').map((b) => b.closest('label')?.textContent);
    expect(boxes).toEqual(['FAIL(3)', 'WARNING(12)', 'PASS(30)']);
    expect(screen.getByText('Showing every result.')).toBeDefined();
  });

  it('Hide warnings hides only the warnings and says how many', () => {
    render(<Harness counts={{ PASS: 30, WARNING: 12, FAIL: 3 }} />);
    fireEvent.click(screen.getByRole('button', { name: 'Hide warnings' }));
    expect(screen.getByTestId('hidden').textContent).toBe('WARNING');
    expect(screen.getByText(/12 results are hidden by this filter/)).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Show warnings' }));
    expect(screen.getByTestId('hidden').textContent).toBe('');
  });

  it('offers nothing when there is only one kind of result', () => {
    const { container } = render(<Harness counts={{ PASS: 30 }} />);
    expect(container.querySelector('[data-testid="verdict-filter"]')).toBeNull();
  });
});

describe('resultCounts', () => {
  it('counts a failure card and its table row as one result', () => {
    const results = {
      tableResults: [
        { tableId: 's.a', verdict: 'FAIL' },
        { tableId: 's.b', verdict: 'PASS' },
      ],
      failures: [
        { id: 'c1', tableId: 's.a', verdict: 'FAIL' },
        { id: 'c2', tableId: 'database.encoding', verdict: 'WARNING' },
      ],
    } as unknown as Results;
    expect(resultCounts(results)).toEqual({ FAIL: 1, PASS: 1, WARNING: 1 });
  });
});
