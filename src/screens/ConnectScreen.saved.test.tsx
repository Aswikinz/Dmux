import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { createMockEngine } from '../api/mockEngine';
import { ThemeProvider } from '../design/theme';
import type { Engine } from '../api/engine';
import type { ConnectionProfile } from '../api/contract';
import { ConnectScreen, defaultOldProfile, defaultNewProfile } from './ConnectScreen';

let latest: { old: ConnectionProfile; neu: ConnectionProfile } | null = null;

function Harness({ engine }: { engine: Engine }) {
  const [oldProfile, setOld] = useState(defaultOldProfile());
  const [newProfile, setNew] = useState(defaultNewProfile());
  latest = { old: oldProfile, neu: newProfile };
  return (
    <ConnectScreen
      engine={engine}
      oldProfile={oldProfile}
      newProfile={newProfile}
      onChangeOld={setOld}
      onChangeNew={setNew}
      onNext={() => {}}
    />
  );
}

function show(engine: Engine) {
  return render(
    <ThemeProvider>
      <Harness engine={engine} />
    </ThemeProvider>,
  );
}

const saved: ConnectionProfile = {
  id: 'p-prod',
  label: 'Production',
  host: 'db.example.org',
  port: 6543,
  database: 'bank',
  user: 'reader',
  encryption: 'verify',
};

async function engineWithSaved(withPassword: boolean): Promise<Engine> {
  const engine = createMockEngine();
  await engine.saveProfile(saved, withPassword ? 'typed-once' : undefined);
  return engine;
}

describe('Connect screen: saved connections', () => {
  it('offers nothing to pick when no connection is saved', async () => {
    show(createMockEngine());
    const select = (await screen.findByTestId('Old database-saved-select')) as HTMLSelectElement;
    expect(select.disabled).toBe(true);
    expect(within(select).getByText('No saved connections yet')).toBeDefined();
  });

  it('fills the form from a saved connection and says a password is saved', async () => {
    show(await engineWithSaved(true));
    const select = (await screen.findByTestId('Old database-saved-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    fireEvent.change(select, { target: { value: 'p-prod' } });
    await waitFor(() => expect(latest?.old.host).toBe('db.example.org'));
    expect(latest?.old).toMatchObject({ id: 'p-prod', port: 6543, database: 'bank', user: 'reader' });
    expect(await screen.findByText('A password is saved for this connection.')).toBeDefined();
    // The password is never put in the form.
    expect((screen.getByTestId('Old database-password') as HTMLInputElement).value).toBe('');
  });

  it('says when no password is saved', async () => {
    show(await engineWithSaved(false));
    const select = (await screen.findByTestId('New database-saved-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    fireEvent.change(select, { target: { value: 'p-prod' } });
    expect(await screen.findByText(/No password is saved for this connection/)).toBeDefined();
  });

  it('will not use one saved connection for both databases', async () => {
    show(await engineWithSaved(true));
    const oldSelect = (await screen.findByTestId('Old database-saved-select')) as HTMLSelectElement;
    await waitFor(() => expect(oldSelect.disabled).toBe(false));
    fireEvent.change(oldSelect, { target: { value: 'p-prod' } });
    await waitFor(() => expect(latest?.old.id).toBe('p-prod'));
    const newSelect = screen.getByTestId('New database-saved-select') as HTMLSelectElement;
    fireEvent.change(newSelect, { target: { value: 'p-prod' } });
    expect(await screen.findByText(/already used for the other database/)).toBeDefined();
    expect(latest?.neu.id).toBe('new-profile');
    expect(latest?.neu.host).toBe('');
  });

  it('deletes a saved connection only after a confirm step', async () => {
    const engine = await engineWithSaved(true);
    show(engine);
    const select = (await screen.findByTestId('Old database-saved-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    fireEvent.change(select, { target: { value: 'p-prod' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Delete connection' }));
    // Backing out keeps it.
    fireEvent.click(screen.getByRole('button', { name: 'Keep it' }));
    expect((await engine.listProfiles()).length).toBe(1);

    fireEvent.click(screen.getByRole('button', { name: 'Delete connection' }));
    fireEvent.click(screen.getByRole('button', { name: 'Yes, delete it' }));
    expect(await screen.findByText(/Deleted the saved connection "Production"/)).toBeDefined();
    expect(await engine.listProfiles()).toEqual([]);
    expect(await engine.profileHasPassword('p-prod')).toBe(false);
    expect(screen.queryByTestId('Old database-saved-card')).toBeNull();
    await waitFor(() =>
      expect((screen.getByTestId('Old database-saved-select') as HTMLSelectElement).disabled).toBe(true),
    );
  });

  async function fillAndTestOld(engine: Engine, password?: string) {
    fireEvent.change(await screen.findByTestId('Old database-host'), { target: { value: 'h1' } });
    fireEvent.change(screen.getByTestId('Old database-database'), { target: { value: 'd' } });
    fireEvent.change(screen.getByTestId('Old database-user'), { target: { value: 'u' } });
    if (password) {
      fireEvent.change(screen.getByTestId('Old database-password'), { target: { value: password } });
    }
    fireEvent.click(screen.getAllByRole('button', { name: 'Test connection' })[0]);
    await screen.findByText('Server version:');
    return engine;
  }

  it('saves a connection automatically after a successful test and says so', async () => {
    const engine = createMockEngine();
    show(engine);
    await fillAndTestOld(engine, 'typed-once');
    const card = await screen.findByTestId('Old database-saved-card');
    expect(within(card).getByText('Saved')).toBeDefined();
    expect(within(card).getByText(/Last tested 20\d\d-\d\d-\d\d \d\d:\d\d UTC/)).toBeDefined();
    expect(within(card).getByText('Password is in your system keychain.')).toBeDefined();
    expect((await engine.listProfiles()).map((p) => p.host)).toEqual(['h1']);
    expect(await engine.profileHasPassword('old-profile')).toBe(true);
    // The typed password is cleared from the form.
    expect((screen.getByTestId('Old database-password') as HTMLInputElement).value).toBe('');
  });

  it('does not save a connection whose test failed', async () => {
    const engine = createMockEngine();
    show(engine);
    fireEvent.change(await screen.findByTestId('Old database-host'), { target: { value: 'error' } });
    fireEvent.click(screen.getAllByRole('button', { name: 'Test connection' })[0]);
    await screen.findByText('Connection failed');
    expect(await engine.listProfiles()).toEqual([]);
    expect(screen.queryByTestId('Old database-saved-card')).toBeNull();
  });

  it('says the password is only kept for this session when there is no keychain', async () => {
    const base = createMockEngine();
    const engine: Engine = {
      ...base,
      saveProfile: async (profile, password) => ({
        ...(await base.saveProfile(profile, password)),
        warning: 'No system keychain: Dmux will keep this password in memory until you close the app.',
      }),
    };
    show(engine);
    await fillAndTestOld(engine, 'typed-once');
    const card = await screen.findByTestId('Old database-saved-card');
    expect(within(card).getByText(/kept in memory only until you close the app/)).toBeDefined();
  });

  it('retries the connection without retyping the password', async () => {
    const engine = createMockEngine();
    const test = vi.spyOn(engine, 'testConnection');
    show(engine);
    await fillAndTestOld(engine, 'typed-once');
    const card = await screen.findByTestId('Old database-saved-card');
    fireEvent.click(within(card).getByRole('button', { name: 'Retry connection' }));
    await waitFor(() => expect(test).toHaveBeenCalledTimes(2));
    expect(test.mock.calls[1][1]).toBeUndefined();
  });

  it('shows unsaved changes, and refuses to test changed details without a password', async () => {
    const engine = createMockEngine();
    const test = vi.spyOn(engine, 'testConnection');
    show(engine);
    await fillAndTestOld(engine, 'typed-once');
    fireEvent.change(screen.getByTestId('Old database-host'), { target: { value: 'other-host' } });
    const card = await screen.findByTestId('Old database-saved-card');
    expect(within(card).getByText(/with unsaved changes/)).toBeDefined();
    fireEvent.click(within(card).getByRole('button', { name: 'Retry connection' }));
    expect(await screen.findByText(/You changed the connection details/)).toBeDefined();
    expect(test).toHaveBeenCalledTimes(1);
  });

  it('Edit moves to the saved details', async () => {
    const engine = createMockEngine();
    show(engine);
    await fillAndTestOld(engine, 'typed-once');
    fireEvent.click(within(await screen.findByTestId('Old database-saved-card')).getByRole('button', { name: 'Edit' }));
    expect(document.activeElement).toBe(screen.getByTestId('Old database-host'));
  });

  it('shows a newly saved connection in the list', async () => {
    const engine = createMockEngine();
    show(engine);
    fireEvent.change(await screen.findByTestId('Old database-host'), { target: { value: 'h1' } });
    fireEvent.change(screen.getByTestId('Old database-database'), { target: { value: 'd' } });
    fireEvent.change(screen.getByTestId('Old database-user'), { target: { value: 'u' } });
    fireEvent.click(screen.getAllByRole('button', { name: 'Save' })[0]);
    await waitFor(() =>
      expect((screen.getByTestId('New database-saved-select') as HTMLSelectElement).disabled).toBe(false),
    );
    expect(within(screen.getByTestId('New database-saved-select')).getByText(/u@h1:5432\/d/)).toBeDefined();
  });
});
