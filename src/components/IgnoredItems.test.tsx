import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { IgnoredItems } from './IgnoredItems';

const NOT_SELECTED = 'not selected for this run';

describe('IgnoredItems', () => {
  it('counts left-out tables apart, but lists a left-out column beside the person’s own ignores', () => {
    render(
      <IgnoredItems
        ignored={[
          { id: 'a', name: 'core.archive', reason: 'archived before the migration' },
          { id: 'b', name: 'core.loans', reason: NOT_SELECTED },
          { id: 'c', name: 'core.fees', reason: NOT_SELECTED },
          { id: 'd', name: 'core.accounts.nickname', reason: NOT_SELECTED },
        ]}
      />,
    );
    // Two whole tables were outside the run: counted, listed in their own report.
    expect(screen.getByTestId('not-in-run').textContent).toContain('2 tables were not part of this run');
    // The column was in a checked table, so it is listed with the other items.
    expect(screen.getByText('2 items, each with its reason')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Show ignored items' }));
    expect(screen.getByText('core.accounts.nickname')).toBeDefined();
    expect(screen.getByText('core.archive')).toBeDefined();
    expect(screen.queryByText('core.loans')).toBeNull();
  });
});
