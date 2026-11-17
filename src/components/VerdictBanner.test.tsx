import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { VerdictBanner, qualifiedVerdictLabel } from './VerdictBanner';
import { VERDICT_WORDS } from '../design/StatusBadge';
import type { Verdict } from '../api/contract';

describe('VerdictBanner', () => {
  it.each(Object.keys(VERDICT_WORDS) as Verdict[])('states %s in words with an icon', (v) => {
    const { container, unmount } = render(<VerdictBanner verdict={v} />);
    expect(screen.getByTestId('verdict-word').textContent).toBe(VERDICT_WORDS[v]);
    expect(container.querySelector('svg')).toBeTruthy();
    unmount();
  });

  it('never shows PASS for an unrecognised verdict', () => {
    render(<VerdictBanner verdict={'BOGUS' as Verdict} />);
    expect(screen.getByText('Result: UNKNOWN')).toBeDefined();
    expect(screen.queryByText(/PASS/)).toBeNull();
  });

  it('qualifies a PASS that used options or cannot vouch for them', () => {
    expect(qualifiedVerdictLabel({ verdict: 'PASS', optionsApplied: true, optionsRecorded: true })).toBe(
      'PASS (with options)',
    );
    expect(
      qualifiedVerdictLabel({ verdict: 'PASS', optionsApplied: false, optionsRecorded: false }),
    ).toMatch(/options not recorded/);
    expect(
      qualifiedVerdictLabel({ verdict: 'PASS', optionsApplied: false, optionsRecorded: true }),
    ).toBeUndefined();
    expect(
      qualifiedVerdictLabel({ verdict: 'FAIL', optionsApplied: true, optionsRecorded: false }),
    ).toBeUndefined();
  });
});
