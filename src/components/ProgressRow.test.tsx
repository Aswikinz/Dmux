import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ProgressRow } from './ProgressRow';
import type { TableProgress } from '../api/contract';

describe('ProgressRow', () => {
  it('shows the table name, progress and a word badge', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000001',
      schemaName: 'demo_bank',
      tableName: 'account_master',
      state: 'done',
      progressPercent: 100,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('demo_bank.account_master')).toBeDefined();
    expect(screen.getByText('Pass')).toBeDefined();
  });

  it('maps failed to a Fail badge', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000002',
      schemaName: 'demo_bank',
      tableName: 'account_txn',
      state: 'failed',
      progressPercent: 0,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('Fail')).toBeDefined();
  });

  it('maps changed-during-check to its badge', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000003',
      schemaName: 'demo_bank',
      tableName: 'acct_cr',
      state: 'changed-during-check',
      progressPercent: 0,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('Changed during check')).toBeDefined();
  });

  it('shows eta while running', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000004',
      schemaName: 'demo_bank',
      tableName: 'bal_hist',
      state: 'running',
      progressPercent: 40,
      etaSeconds: 12,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText(/12s left/)).toBeDefined();
  });
});