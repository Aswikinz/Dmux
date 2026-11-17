import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, beforeEach } from 'vitest';
import App from './App';

describe('App', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('renders the app shell with the step sidebar and the Connect screen', () => {
    render(<App />);

    // The brand sits in the sidebar.
    expect(screen.getByText('Dmux')).toBeDefined();

    // The guided step navigation is present and ordered.
    const nav = screen.getByRole('navigation', { name: 'Progress' });
    expect(nav).toBeDefined();

    // The first step is the Connect screen.
    expect(screen.getByRole('heading', { level: 2, name: 'Connect' })).toBeDefined();

    // Both connection panels are shown before any step completes.
    expect(screen.getByText('Old database')).toBeDefined();
    expect(screen.getByText('New database')).toBeDefined();
  });

  it('offers the theme and reduced-effects toggles', () => {
    render(<App />);
    expect(
      screen.getByRole('button', { name: /switch to (light|dark) mode/i }),
    ).toBeDefined();
    expect(
      screen.getByRole('button', { name: /(enable|disable) reduced effects/i }),
    ).toBeDefined();
  });

  it('toggles dark mode and reduced effects', () => {
    render(<App />);
    const darkButton = screen.getByRole('button', { name: /switch to dark mode/i });
    fireEvent.click(darkButton);
    expect(
      screen.getByRole('button', { name: /switch to light mode/i }),
    ).toBeDefined();

    const effects = screen.getByRole('button', { name: /enable reduced effects/i });
    fireEvent.click(effects);
    expect(
      screen.getByRole('button', { name: /disable reduced effects/i }),
    ).toBeDefined();
  });

  it('shows the sidebar toggle on small screens (mobile)', () => {
    render(<App />);
    // The mobile menu button is always rendered, hidden via responsive classes.
    expect(screen.getByRole('button', { name: 'Toggle sidebar' })).toBeDefined();
  });
});