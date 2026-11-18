import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ProgressRow } from './ProgressRow';
import type { TableProgress } from '../api/contract';

describe('ProgressRow', () => {
  it('shows the table name, progress and a neutral state word (Scan finished, never PASS)', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000001',
      schemaName: 'demo_bank',
      tableName: 'account_master',
      state: 'done',
      progressPercent: 100,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('demo_bank.account_master')).toBeDefined();
    expect(screen.getByText('Scan finished')).toBeDefined();
    // A finished scan is not a verified PASS.
    expect(screen.queryByText('PASS')).toBeNull();
    expect(screen.queryByText('Done')).toBeNull();
  });

  it('a done table never shows a PASS badge, even if it later differs', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000005',
      schemaName: 'demo_bank',
      tableName: 'acct_diff',
      state: 'done',
      progressPercent: 100,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('Scan finished')).toBeDefined();
    expect(screen.queryByText('PASS')).toBeNull();
    expect(screen.queryByText('Done')).toBeNull();
  });

  it('maps a failed scan to Scan failed, not the data verdict FAIL', () => {
    const progress: TableProgress = {
      tableId: 'tbl-000002',
      schemaName: 'demo_bank',
      tableName: 'account_txn',
      state: 'failed',
      progressPercent: 0,
    };
    render(<ProgressRow progress={progress} />);
    expect(screen.getByText('Scan failed')).toBeDefined();
    expect(screen.queryByText('FAIL')).toBeNull();
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
    expect(screen.getByText('CHANGED DURING CHECK')).toBeDefined();
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
    expect(screen.getByText('about 12 seconds left')).toBeDefined();
  });

  it('says Not scanned (never Queued) for a table the ended run did not read', () => {
    render(
      <ProgressRow
        progress={{
          tableId: 'structure.client_notes',
          schemaName: 'structure',
          tableName: 'client_notes',
          state: 'not-scanned',
          progressPercent: 0,
        }}
      />,
    );
    expect(screen.getByText('Not scanned')).toBeDefined();
    expect(screen.queryByText('Queued')).toBeNull();
  });
});

