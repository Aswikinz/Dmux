import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import { VerdictFilter } from './VerdictFilter';
import { coverageLabel, kindCounts, resultCounts, resultItems } from '../screens/ResultsScreen';
import type { IssueKind, Results, Verdict } from '../api/contract';

function Harness({
  counts,
  kinds = {},
}: {
  counts: Partial<Record<Verdict, number>>;
  kinds?: Partial<Record<IssueKind, number>>;
}) {
  const [hidden, setHidden] = useState<Set<Verdict>>(new Set());
  const [hiddenKinds, setHiddenKinds] = useState<Set<IssueKind>>(new Set());
  const total =
    Object.entries(counts).reduce((n, [v, c]) => n + (hidden.has(v as Verdict) ? (c ?? 0) : 0), 0) +
    Object.entries(kinds).reduce((n, [k, c]) => n + (hiddenKinds.has(k as IssueKind) ? (c ?? 0) : 0), 0);
  return (
    <>
      <VerdictFilter
        counts={counts}
        hidden={hidden}
        onChange={setHidden}
        kindCounts={kinds}
        hiddenKinds={hiddenKinds}
        onKindsChange={setHiddenKinds}
        hiddenTotal={total}
      />
      <output data-testid="hidden">{[...hidden].sort().join(',')}</output>
      <output data-testid="hidden-kinds">{[...hiddenKinds].sort().join(',')}</output>
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

  it('lists the kinds of problem data first, and "Only data differences" hides the rest', () => {
    render(
      <Harness counts={{ FAIL: 9 }} kinds={{ other: 2, structure: 5, data: 2 }} />,
    );
    const labels = screen
      .getAllByRole('checkbox')
      .map((b) => b.closest('label')?.textContent);
    expect(labels).toEqual(['Data differences(2)', 'Structure differences(5)', 'Other reasons(2)']);
    fireEvent.click(screen.getByRole('button', { name: 'Only data differences' }));
    expect(screen.getByTestId('hidden-kinds').textContent).toBe('other,structure');
    expect(screen.getByText(/7 results are hidden by this filter/)).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Show every kind' }));
    expect(screen.getByTestId('hidden-kinds').textContent).toBe('');
  });

  it('offers nothing when there is only one kind of result', () => {
    const { container } = render(<Harness counts={{ PASS: 30 }} />);
    expect(container.querySelector('[data-testid="verdict-filter"]')).toBeNull();
  });
});

describe('resultCounts and kindCounts', () => {
  const results = {
    tableResults: [
      { tableId: 's.a', verdict: 'FAIL', kind: 'data' },
      { tableId: 's.b', verdict: 'PASS' },
      { tableId: 's.c', verdict: 'FAIL', kind: 'structure' },
    ],
    failures: [
      { id: 'c1', tableId: 's.a', verdict: 'FAIL', kind: 'data' },
      { id: 'c2', tableId: 'database.encoding', verdict: 'WARNING', kind: 'structure' },
      { id: 'c3', tableId: 's.c', verdict: 'FAIL', kind: 'structure' },
    ],
  } as unknown as Results;

  it('counts a failure card and its table row as one result', () => {
    expect(resultItems(results)).toHaveLength(4);
    expect(resultCounts(results)).toEqual({ FAIL: 2, PASS: 1, WARNING: 1 });
  });

  it('counts kinds only for failures', () => {
    expect(kindCounts(results)).toEqual({ data: 1, structure: 2 });
  });
});

describe('coverageLabel', () => {
  it('names the tables in scope that were not checked, so the parts add up', () => {
    const c = { totalTables: 34, fullValuesTables: 32, countsTables: 0, structureTables: 0, ignoredTables: 0 };
    expect(coverageLabel(c)).toBe(
      '32 of 34 tables fully checked, 0 counts only, 0 structure only, 0 ignored, 2 not checked',
    );
    expect(coverageLabel({ ...c, fullValuesTables: 34 })).toBe(
      '34 of 34 tables fully checked, 0 counts only, 0 structure only, 0 ignored',
    );
  });
});

