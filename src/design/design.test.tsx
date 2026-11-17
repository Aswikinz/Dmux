import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Button } from './Button';
import { Panel } from './Panel';
import { StatusBadge } from './StatusBadge';
import { ErrorCard } from './ErrorCard';
import { Tooltip } from './Tooltip';
import { Input } from './Input';
import { Select } from './Select';
import { StepSidebar, type Step } from './StepSidebar';
import { ThemeProvider, useTheme } from './theme';

describe('Button', () => {
  it('renders children and fires clicks', async () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Go</Button>);
    fireEvent.click(screen.getByRole('button', { name: 'Go' }));
    expect(onClick).toHaveBeenCalled();
  });
});

describe('Panel', () => {
  it('renders children within a theme provider', () => {
    render(
      <ThemeProvider>
        <Panel>content</Panel>
      </ThemeProvider>,
    );
    expect(screen.getByText('content')).toBeDefined();
  });

  it('uses the glass style by default and solid in reduced effects', () => {
    const Glass = () => {
      const { reducedEffects } = useTheme();
      return <Panel>{reducedEffects ? 'solid' : 'glass'}</Panel>;
    };
    const { unmount } = render(
      <ThemeProvider>
        <Glass />
      </ThemeProvider>,
    );
    expect(screen.getByText('glass')).toBeDefined();
    unmount();
    // Seed the reduced-effects preference, then mount a fresh provider so the
    // useState initializer reads it.
    localStorage.setItem('dmux-reduced-effects', 'true');
    render(
      <ThemeProvider>
        <Glass />
      </ThemeProvider>,
    );
    expect(screen.getByText('solid')).toBeDefined();
    localStorage.removeItem('dmux-reduced-effects');
  });
});

describe('StatusBadge', () => {
  const verdictLabels: Record<string, string> = {
    PASS: 'Pass',
    FAIL: 'Fail',
    WIDELY_DIFFERENT: 'Widely different',
    NEEDS_REVIEW: 'Needs review',
    CHANGED_DURING_CHECK: 'Changed during check',
    TOOL_FAULT: 'Tool fault',
    WARNING: 'Warning',
  };

  it.each(Object.keys(verdictLabels))('renders a word and icon for %s', (status) => {
    const { container } = render(<StatusBadge status={status as never} />);
    expect(screen.getByText(verdictLabels[status])).toBeDefined();
    expect(container.querySelector('svg')).toBeTruthy();
  });

  it('renders a neutral Unknown state for an unrecognised status, never Pass or Idle', () => {
    const { container } = render(<StatusBadge status={'bogus' as never} />);
    expect(screen.getByText('Unknown')).toBeDefined();
    expect(screen.queryByText('Pass')).toBeNull();
    expect(screen.queryByText('Idle')).toBeNull();
    expect(container.querySelector('svg')).toBeTruthy();
  });
});

describe('ErrorCard', () => {
  it('renders its title and alerts', () => {
    render(<ErrorCard title="Something failed">Detailed message</ErrorCard>);
    expect(screen.getByRole('alert')).toBeDefined();
    expect(screen.getByText('Something failed')).toBeDefined();
    expect(screen.getByText('Detailed message')).toBeDefined();
  });
});

describe('Tooltip', () => {
  it('shows the explanation on focus', () => {
    render(<Tooltip label="Superuser" explanation="Can run any command">Superuser login</Tooltip>);
    fireEvent.focus(screen.getByRole('button', { name: 'Superuser login' }));
    expect(screen.getByText('Superuser')).toBeDefined();
    expect(screen.getByText('Can run any command')).toBeDefined();
  });

  it('shows and hides on mouse enter and leave', () => {
    render(<Tooltip label="Dsn" explanation="A connection string">source DSN</Tooltip>);
    const trigger = screen.getByRole('button', { name: 'source DSN' });
    fireEvent.mouseEnter(trigger);
    expect(screen.getByText('A connection string')).toBeDefined();
    fireEvent.mouseLeave(trigger);
    expect(screen.queryByText('A connection string')).toBeNull();
  });
});

describe('Input', () => {
  it('renders a labelled input', () => {
    render(<Input label="Host" />);
    expect(screen.getByLabelText('Host')).toBeDefined();
  });
});

describe('Select', () => {
  it('renders options', () => {
    render(
      <Select
        label="Preset"
        options={[{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }]}
      />,
    );
    expect(screen.getByLabelText('Preset')).toBeDefined();
    expect(screen.getByRole('option', { name: 'Alpha' })).toBeDefined();
  });
});

describe('StepSidebar', () => {
  it('marks current and completed steps', () => {
    const steps: { id: Step; label: string }[] = [
      { id: 'connect', label: 'Connect' },
      { id: 'structure', label: 'Compare structure' },
    ];
    render(
      <StepSidebar
        steps={steps}
        current="structure"
        completed={new Set(['connect'])}
        onSelect={() => {}}
      />,
    );
    const nav = screen.getByRole('navigation', { name: 'Progress' });
    expect(nav).toBeDefined();
    const current = screen.getByRole('button', { name: /compare structure/i });
    expect(current.getAttribute('aria-current')).toBe('step');
  });
});
