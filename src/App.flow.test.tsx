import { render, screen, fireEvent, within } from '@testing-library/react';
import { describe, it } from 'vitest';
import App from './App';
import type { FixtureName } from './api/mockEngine';

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
      await screen.findByText('Estimate:');

      // Choose checks -> Run
      fireEvent.click(cta('Run'));
      await screen.findByText('Finished');

      // Run -> Results
      fireEvent.click(cta('See results'));
      await screen.findByText(/tables fully checked/);

      // Results -> Report
      fireEvent.click(cta('Report'));
      await screen.findByRole('button', { name: /export pdf/i });
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
      await screen.findByText('Estimate:');
      fireEvent.click(cta('Run'));
      await screen.findByText('Finished');
      fireEvent.click(cta('See results'));
      await screen.findByText('Failures');
    } finally {
      cleanup();
    }
  });
});
