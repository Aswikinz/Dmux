import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { createMockEngine, fixtures } from '../api/mockEngine';
import { ThemeProvider } from '../design/theme';
import type { Engine } from '../api/engine';
import { HistoryScreen, formatWhen } from './HistoryScreen';

function show(engine: Engine, onOpenResults: (r: unknown) => void = () => {}) {
  return render(
    <ThemeProvider>
      <HistoryScreen engine={engine} onOpenResults={onOpenResults} />
    </ThemeProvider>,
  );
}

describe('HistoryScreen', () => {
  it('lists the finished runs with date, verdict word, coverage and databases', async () => {
    show(createMockEngine());
    expect(await screen.findByText('2026-11-16 09:48 UTC')).toBeDefined();
    expect(screen.getByText('FAIL')).toBeDefined();
    expect(screen.getByText('PASS')).toBeDefined();
    expect(screen.getAllByText(/tables fully checked/).length).toBe(2);
    expect(screen.getAllByText('Old database to New database').length).toBe(2);
  });

  it('says so when there is no history', async () => {
    const engine = createMockEngine();
    await engine.purgeHistory();
    show(engine);
    expect(await screen.findByText(/No finished checks yet/)).toBeDefined();
    expect((screen.getByRole('button', { name: 'Delete all history' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('opens a history entry and hands its verified results to the app', async () => {
    const onOpen = vi.fn();
    show(createMockEngine(), onOpen);
    const buttons = await screen.findAllByRole('button', { name: /Open results from/ });
    fireEvent.click(buttons[0]);
    await waitFor(() => expect(onOpen).toHaveBeenCalledTimes(1));
    const opened = onOpen.mock.calls[0][0] as { verdict: string; runId: string };
    expect(opened.verdict).toBe('FAIL');
    expect(opened.runId).toContain('opened');
  });

  it('shows a plain error when a stored entry cannot be opened', async () => {
    const base = createMockEngine();
    const engine: Engine = {
      ...base,
      openHistoryEntry: () =>
        Promise.reject(new Error('This stored results file has been changed.')),
    };
    const onOpen = vi.fn();
    show(engine, onOpen);
    fireEvent.click((await screen.findAllByRole('button', { name: /Open results from/ }))[0]);
    expect(await screen.findByRole('alert')).toBeDefined();
    expect(screen.getByText(/has been changed/)).toBeDefined();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('opens a results file and shows it', async () => {
    const onOpen = vi.fn();
    show(createMockEngine(), onOpen);
    fireEvent.click(await screen.findByRole('button', { name: 'Open a results file' }));
    await waitFor(() => expect(onOpen).toHaveBeenCalledTimes(1));
  });

  it('shows nothing and says so when the open dialog is dismissed', async () => {
    const onOpen = vi.fn();
    show(createMockEngine({ openFile: 'cancelled' }), onOpen);
    fireEvent.click(await screen.findByRole('button', { name: 'Open a results file' }));
    expect(await screen.findByText('Nothing was opened.')).toBeDefined();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('refuses a changed results file with a plain error and shows nothing', async () => {
    const onOpen = vi.fn();
    show(createMockEngine({ openFile: 'tampered' }), onOpen);
    fireEvent.click(await screen.findByRole('button', { name: 'Open a results file' }));
    expect(await screen.findByText(/has been changed since Dmux wrote it/)).toBeDefined();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('shows the run log as text and saves it', async () => {
    show(createMockEngine());
    fireEvent.click((await screen.findAllByRole('button', { name: /Show the run log/ }))[0]);
    const text = await screen.findByTestId('run-log-text');
    expect(text.textContent).toContain('Dmux run log');
    expect(text.textContent).toContain('verdict: FAIL');
    fireEvent.click(screen.getByRole('button', { name: 'Save run log' }));
    expect(await screen.findByText(/Run log saved to chosen-location/)).toBeDefined();
  });

  it('reports a dismissed or failed run-log save', async () => {
    const base = createMockEngine();
    let mode: 'cancel' | 'fail' = 'cancel';
    const engine: Engine = {
      ...base,
      saveHistoryLog: () =>
        Promise.resolve(
          mode === 'cancel' ? { ok: false, cancelled: true } : { ok: false, error: 'Disk full.' },
        ),
    };
    show(engine);
    fireEvent.click((await screen.findAllByRole('button', { name: /Show the run log/ }))[0]);
    await screen.findByTestId('run-log-text');
    fireEvent.click(screen.getByRole('button', { name: 'Save run log' }));
    expect(await screen.findByText(/Saving cancelled/)).toBeDefined();
    mode = 'fail';
    fireEvent.click(screen.getByRole('button', { name: 'Save run log' }));
    expect(await screen.findByText('Disk full.')).toBeDefined();
  });

  it('asks before deleting history, can be backed out of, then deletes everything', async () => {
    const engine = createMockEngine();
    const purge = vi.spyOn(engine, 'purgeHistory');
    show(engine);
    fireEvent.click(await screen.findByRole('button', { name: 'Delete all history' }));
    const confirm = screen.getByRole('group', { name: 'Confirm deleting history' });
    expect(within(confirm).getByText(/cannot be undone/)).toBeDefined();
    expect(purge).not.toHaveBeenCalled();
    fireEvent.click(within(confirm).getByRole('button', { name: 'Keep my history' }));
    expect(screen.queryByRole('group', { name: 'Confirm deleting history' })).toBeNull();
    expect(purge).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Delete all history' }));
    fireEvent.click(screen.getByRole('button', { name: 'Yes, delete everything' }));
    expect(await screen.findByText('Deleted 2 stored checks and their results files.')).toBeDefined();
    expect(purge).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/No finished checks yet/)).toBeDefined();
  });

  it('never shows a bare PASS for a run with options', async () => {
    const base = createMockEngine();
    const engine: Engine = {
      ...base,
      listHistory: async () => [
        { ...(await base.listHistory())[1], optionsApplied: true },
      ],
    };
    show(engine);
    expect(await screen.findByText('PASS (with options)')).toBeDefined();
  });

  it('formats dates unambiguously and survives a bad date', () => {
    expect(formatWhen('2026-11-16T09:48:30Z')).toBe('2026-11-16 09:48 UTC');
    expect(formatWhen('not a date')).toBe('Unknown');
    expect(fixtures.fail.verdict).toBe('FAIL');
  });
});
