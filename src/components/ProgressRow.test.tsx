import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ProgressRow } from './ProgressRow';
import type { TableProgress } from '../api/contract';

describe('ProgressRow', () => {
  it('shows the table name, progress and a state word (Done, not Pass)', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000001',
      schemaName: 'demo_bank',
      tableName: 'account_master',
      state: 'done',
      progressPercent: 100,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('demo_bank.account_master')).toBeDefined();
    expect(screen.getByText('Done')).toBeDefined();
    // A finished scan is not a verified PASS.
    expect(screen.queryByText('Pass')).toBeNull();
  });

  it('a done table never shows a Pass badge, even if it later differs', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000005',
      schemaName: 'demo_bank',
      tableName: 'acct_diff',
      state: 'done',
      progressPercent: 100,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('Done')).toBeDefined();
    expect(screen.queryByText('Pass')).toBeNull();
  });

  it('maps a failed scan to Failed, not data Fail', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000002',
      schemaName: 'demo_bank',
      tableName: 'account_txn',
      state: 'failed',
      progressPercent: 0,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('Failed')).toBeDefined();
    expect(screen.queryByText('Fail')).toBeNull();
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