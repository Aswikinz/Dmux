import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import App from './App';

describe('App', () => {
  it('renders the Phase 0 placeholder screen', () => {
    render(<App />);
    expect(
      screen.getByRole('heading', {
        level: 1,
        name: 'Dmux — migration verification',
      }),
    ).toBeDefined();
    expect(screen.getByText('Phase 0 skeleton')).toBeDefined();
  });
});
