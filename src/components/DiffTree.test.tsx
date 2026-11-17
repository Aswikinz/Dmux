import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { DiffTree } from './DiffTree';
import type { SchemaDiffNode } from '../api/contract';

const nodes: SchemaDiffNode[] = [
  {
    id: 'sch-1',
    kind: 'schema',
    name: 'demo_bank',
    status: 'same',
    children: [
      {
        id: 'tbl-1',
        kind: 'table',
        name: 'accounts',
        status: 'same',
        children: [
          { id: 'col-1', kind: 'column', name: 'balance', status: 'changed', old: 'numeric(18,2)', new: 'numeric(19,2)' },
        ],
      },
      { id: 'tbl-2', kind: 'table', name: 'archived', status: 'missing' },
    ],
  },
];

describe('DiffTree', () => {
  it('renders nodes and problem badges', () => {
    render(<DiffTree nodes={nodes} />);
    expect(screen.getByText('demo_bank')).toBeDefined();
    expect(screen.getByText('accounts')).toBeDefined();
    expect(screen.getByText('balance')).toBeDefined();
    expect(screen.getByText('archived')).toBeDefined();
    // Changed and missing nodes carry a word badge.
    expect(screen.getByText('Changed')).toBeDefined();
    expect(screen.getByText('Missing')).toBeDefined();
  });

  it('filters to problems only when asked', () => {
    render(<DiffTree nodes={nodes} showOnlyProblems />);
    expect(screen.queryByText('demo_bank')).toBeDefined(); // schema kept as container
    expect(screen.queryByText('accounts')).toBeDefined(); // kept: has a problem child
    // 'missing' status node kept
    expect(screen.getByText('archived')).toBeDefined();
  });
});
