import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { ThemeProvider, useTheme } from './theme';

function Probe() {
  const { mode, resolvedMode, reducedEffects, setMode, toggleReducedEffects } = useTheme();
  return (
    <div>
      <span data-testid="mode">{mode}</span>
      <span data-testid="resolved">{resolvedMode}</span>
      <span data-testid="reduced">{String(reducedEffects)}</span>
      <button onClick={() => setMode('dark')}>set-dark</button>
      <button onClick={() => setMode('light')}>set-light</button>
      <button onClick={() => setMode('system')}>set-system</button>
      <button onClick={toggleReducedEffects}>toggle-reduced</button>
    </div>
  );
}

describe('theme', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('defaults to system and light', () => {
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(screen.getByTestId('mode').textContent).toBe('system');
    expect(screen.getByTestId('resolved').textContent).toBe('light');
  });

  it('switches modes through the setter', () => {
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'set-dark' }));
    expect(screen.getByTestId('mode').textContent).toBe('dark');
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    fireEvent.click(screen.getByRole('button', { name: 'set-light' }));
    expect(screen.getByTestId('resolved').textContent).toBe('light');
  });

  it('toggles reduced effects and persists the choice', () => {
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(screen.getByTestId('reduced').textContent).toBe('false');
    fireEvent.click(screen.getByRole('button', { name: 'toggle-reduced' }));
    expect(screen.getByTestId('reduced').textContent).toBe('true');
    expect(localStorage.getItem('dmux-reduced-effects')).toBe('true');
  });

  it('throws when used outside a provider', () => {
    // Silence React's error boundary noise by expecting the throw.
    expect(() => {
      render(<Probe />);
    }).toThrow();
  });

  it('resolves system mode from the preferred colour scheme', () => {
    // A matchMedia stub reporting dark preference.
    const listeners: Array<(e: MediaQueryListEvent) => void> = [];
    const stub = {
      matches: true,
      media: '(prefers-color-scheme: dark)',
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: (_t: string, cb: (e: MediaQueryListEvent) => void) => {
        listeners.push(cb);
      },
      removeEventListener: () => {},
      dispatchEvent: () => false,
    };
    vi.stubGlobal('matchMedia', vi.fn(() => stub));
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>,
    );
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    vi.unstubAllGlobals();
  });
});
