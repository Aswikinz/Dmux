import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { RuleEditor } from './RuleEditor';
import type { MappingRule } from '../api/contract';

const preview = { tablesTotal: 12, tablesByRules: 5, tablesMatched: 10 };

describe('RuleEditor', () => {
  it('adds a schema rename and a prefix rule, then removes one', () => {
    render(<RuleEditor rules={[]} onPreview={async () => preview} onApply={async () => {}} />);
    expect(screen.getByText('No rules yet.')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: /Add schema rename/ }));
    fireEvent.click(screen.getByRole('button', { name: /Add prefix or suffix rule/ }));
    expect(screen.getByLabelText('Rule 1: old schema')).toBeDefined();
    expect(screen.getByLabelText('Rule 2 type')).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Remove rule 1' }));
    expect(screen.queryByLabelText('Rule 1: old schema')).toBeNull();
    expect(screen.getByLabelText('Rule 1 type')).toBeDefined();
  });

  it('says rules never rename columns', () => {
    render(<RuleEditor rules={[]} onPreview={async () => preview} onApply={async () => {}} />);
    expect(screen.getByText(/never rename columns/)).toBeDefined();
  });

  it('previews how many tables the rules pair without applying them', async () => {
    const onPreview = vi.fn(async (rules: MappingRule[]) => (rules.length >= 0 ? preview : preview));
    const onApply = vi.fn(async () => {});
    render(
      <RuleEditor
        rules={[{ kind: 'prefix_replace', pattern: 'tbl_', target: '' }]}
        onPreview={onPreview}
        onApply={onApply}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Preview rules' }));
    await waitFor(() => expect(screen.getByRole('status').textContent).toMatch(/pair 5 of 12 tables/));
    expect(onPreview).toHaveBeenCalledWith([{ kind: 'prefix_replace', pattern: 'tbl_', target: '' }]);
    expect(onApply).not.toHaveBeenCalled();
  });

  it('applies the edited rules, and an edit clears an old preview', async () => {
    const onApply = vi.fn(async (rules: MappingRule[]) => void rules.length);
    render(<RuleEditor rules={[]} onPreview={async () => preview} onApply={onApply} />);
    fireEvent.click(screen.getByRole('button', { name: /Add schema rename/ }));
    // An incomplete rule cannot be previewed or applied.
    expect((screen.getByRole('button', { name: 'Preview rules' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText('Rule 1: old schema'), { target: { value: 'legacy' } });
    fireEvent.change(screen.getByLabelText('Rule 1: new schema'), { target: { value: 'public' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview rules' }));
    await screen.findByRole('status');
    fireEvent.change(screen.getByLabelText('Rule 1: new schema'), { target: { value: 'core' } });
    expect(screen.queryByRole('status')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /Apply rules and rebuild mapping/ }));
    await waitFor(() =>
      expect(onApply).toHaveBeenCalledWith([{ kind: 'schema_rename', pattern: 'legacy', target: 'core' }]),
    );
    await screen.findByText(/rebuilt with these rules/);
  });

  it('shows the backend refusal as it is', async () => {
    render(
      <RuleEditor
        rules={[{ kind: 'schema_rename', pattern: 'a', target: 'b' }]}
        onPreview={async () => preview}
        onApply={async () => {
          throw new Error('schema a is renamed by more than one rule');
        }}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Apply rules/ }));
    expect((await screen.findByRole('alert')).textContent).toMatch(/renamed by more than one rule/);
  });
});
