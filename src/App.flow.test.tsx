import { render, screen, fireEvent, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import App from './App';
import type { FixtureName } from './api/mockEngine';

/** Nothing is chosen on Choose checks until the user picks: choose every
 *  table, then wait for the level controls. */
async function chooseAllTables() {
  fireEvent.click(await screen.findByRole('button', { name: 'Select all' }));
  return screen.findByText('Estimated time:');
}


type Win = typeof window & { __dmuxForcedFixture?: FixtureName };

function renderForced(fixture: FixtureName) {
  (window as Win).__dmuxForcedFixture = fixture;
  render(<App />);
  return () => {
    delete (window as Win).__dmuxForcedFixture;
  };
}

// The primary call-to-action lives inside the <main> panel, while the sidebar
// repeats the same step names as navigation buttons. Scope to <main> so the two
// never collide.
function cta(name: string): HTMLElement {
  const main = screen.getByRole('main');
  return within(main).getByRole('button', { name });
}

async function connectAndAdvance() {
  const testButtons = screen.getAllByRole('button', { name: 'Test connection' });
  fireEvent.click(testButtons[0]);
  await screen.findByText('Server version:');
  fireEvent.click(testButtons[1]);
  await screen.findAllByText('Server version:');
  fireEvent.click(cta('Compare structure'));
  await screen.findByRole('button', { name: 'Map tables' });
}

describe('App full flow', () => {
  it('walks from Connect to Report on a passing run', async () => {
    const cleanup = renderForced('pass');
    try {
      await connectAndAdvance();

      // Compare structure -> Map
      fireEvent.click(cta('Map tables'));
      await screen.findByText(/No items need attention|items need attention/);

      // Map -> Choose checks
      fireEvent.click(cta('Choose checks'));
      await chooseAllTables();

      // Choose checks -> Run
      fireEvent.click(cta('Run'));
      await screen.findByText('The run has finished.');

      // Run -> Results
      fireEvent.click(cta('See results'));
      await screen.findByText(/tables fully checked/);

      // Results -> Report
      fireEvent.click(cta('Report'));
      expect(
        await screen.findByRole('button', { name: /export pdf/i }),
      ).toBeDefined();
      expect(screen.getByRole('button', { name: /export excel/i })).toBeDefined();
    } finally {
      cleanup();
    }
  });

  it('shows failures on a failing run', async () => {
    const cleanup = renderForced('fail');
    try {
      await connectAndAdvance();
      fireEvent.click(cta('Map tables'));
      await screen.findByText(/No items need attention|items need attention/);
      fireEvent.click(cta('Choose checks'));
      await chooseAllTables();
      fireEvent.click(cta('Run'));
      await screen.findByText('The run has finished.');
      fireEvent.click(cta('See results'));
      expect(await screen.findByText('Failures')).toBeDefined();
    } finally {
      cleanup();
    }
  });

  it('after a run, going back and choosing other tables runs again with the new choice', async () => {
    const cleanup = renderForced('pass');
    try {
      await connectAndAdvance();
      fireEvent.click(cta('Map tables'));
      await screen.findByText(/No items need attention|items need attention/);
      fireEvent.click(cta('Choose checks'));
      await chooseAllTables();
      fireEvent.click(cta('Run'));
      await screen.findByText('The run has finished.');
      const firstRows = screen.getAllByText(/^demo_bank\.table_\d{4}$/).length;
      fireEvent.click(cta('See results'));
      await screen.findByText(/tables fully checked/);

      // Back to Choose checks, keep only the archive schema, and run again.
      fireEvent.click(cta('Back'));
      await screen.findByText('The run has finished.');
      fireEvent.click(cta('Back'));
      fireEvent.click(await screen.findByRole('button', { name: 'Clear all' }));
      fireEvent.click(screen.getByRole('checkbox', { name: 'Every table in archive' }));
      fireEvent.click(cta('Run'));

      // A new run: it reads only the 4 chosen tables, not the 40 from before.
      await screen.findByText('The run has finished.');
      const rows = screen.getAllByText(/^demo_bank\.table_\d{4}$/).length;
      expect(firstRows).toBe(40);
      expect(rows).toBe(4);
      // Its results name the 36 tables left out, with the app's reason.
      fireEvent.click(cta('See results'));
      expect((await screen.findByTestId('not-in-run')).textContent).toMatch(
        /36 tables were not part of this run/,
      );
    } finally {
      cleanup();
    }
  });

});
