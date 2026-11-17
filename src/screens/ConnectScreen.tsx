import { useCallback, useEffect, useState } from 'react';
import { describeError } from '../api/tauriEngine';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { Input } from '../design/Input';
import { Select } from '../design/Select';
import { ErrorCard } from '../design/ErrorCard';
import { StatusBadge } from '../design/StatusBadge';
import { Tooltip } from '../design/Tooltip';
import type { Engine } from '../api/engine';
import { formatWhen } from './HistoryScreen';
import type { ConnectionProfile, EncryptionMode, ServerInfo } from '../api/contract';
import { BookmarkCheck, CheckCircle, Database, Pencil, Shield, ShieldAlert, ShieldCheck } from 'lucide-react';

/** Shown when the backend keeps the password for this session only (no keychain). */
export const SESSION_PASSWORD_NOTICE =
  'No system keychain: Dmux will keep this password in memory until you close the app.';

interface ConnectScreenProps {
  engine: Engine;
  oldProfile: ConnectionProfile;
  newProfile: ConnectionProfile;
  onChangeOld: (profile: ConnectionProfile) => void;
  onChangeNew: (profile: ConnectionProfile) => void;
  onNext: () => void;
}

const encryptionOptions: { value: EncryptionMode; label: string }[] = [
  { value: 'verify', label: 'Verify certificate' },
  { value: 'trust', label: 'Trust certificate once' },
  { value: 'allow-plaintext', label: 'Allow unencrypted connection' },
];

function emptyProfile(side: 'old' | 'new'): ConnectionProfile {
  return {
    id: `${side}-profile`,
    label: `${side === 'old' ? 'Old' : 'New'} database`,
    host: '',
    port: 5432,
    database: '',
    user: '',
    encryption: 'verify',
  };
}

export const defaultOldProfile = () => emptyProfile('old');
export const defaultNewProfile = () => emptyProfile('new');

/**
 * Pick a saved connection for one side.  Picking fills the form with the saved
 * details (never a password: the backend keeps that, and only ever sends it to
 * the saved host).  A saved connection can be deleted here, with its password.
 */
function SavedProfilePicker({
  engine,
  title,
  otherProfileId,
  reloadKey,
  onPick,
}: {
  engine: Engine;
  title: string;
  /** The profile id the other side uses; it cannot be used on both sides. */
  otherProfileId: string;
  /** Changes whenever a profile was saved, so the list refreshes. */
  reloadKey: number;
  onPick: (profile: ConnectionProfile, hasPassword: boolean) => void;
}) {
  const [profiles, setProfiles] = useState<ConnectionProfile[]>([]);
  const [selectedId, setSelectedId] = useState('');
  const [hasPassword, setHasPassword] = useState<boolean | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setProfiles(await engine.listProfiles());
    } catch {
      setProfiles([]);
    }
  }, [engine]);

  useEffect(() => {
    void reload();
  }, [reload, reloadKey]);

  async function pick(id: string) {
    setSelectedId(id);
    setMessage(null);
    setHasPassword(null);
    const profile = profiles.find((p) => p.id === id);
    if (!profile) return;
    if (profile.id === otherProfileId) {
      setSelectedId('');
      setMessage('That saved connection is already used for the other database.');
      return;
    }
    const has = await engine.profileHasPassword(profile.id).catch(() => false);
    setHasPassword(has);
    onPick(profile, has);
  }

  const selected = profiles.find((p) => p.id === selectedId);
  const options = [
    { value: '', label: profiles.length === 0 ? 'No saved connections yet' : 'Enter the details by hand' },
    ...profiles.map((p) => ({
      value: p.id,
      label: `${p.label} (${p.user}@${p.host}:${p.port}/${p.database})`,
    })),
  ];

  return (
    <div className="space-y-2" data-testid={`${title}-saved`}>
      <Select
        id={`${title}-saved-select`}
        label={`Saved connections for the ${title.toLowerCase()}`}
        options={options}
        value={selectedId}
        disabled={profiles.length === 0}
        onChange={(e) => void pick(e.target.value)}
        data-testid={`${title}-saved-select`}
      />
      {selected && (
        <div className="space-y-2 text-sm">
          <p className="text-slate-700 dark:text-slate-300" role="status">
            {hasPassword === null
              ? 'Checking for a saved password…'
              : hasPassword
                ? 'A password is saved for this connection.'
                : 'No password is saved for this connection. Type it below, or type it and press Save.'}
          </p>
        </div>
      )}
      {message && (
        <p className="text-sm text-slate-700 dark:text-slate-300" role="status">
          {message}
        </p>
      )}
    </div>
  );
}

export type PasswordKind = 'keychain' | 'session' | 'none';

export interface SavedInfo {
  /** The profile fields as saved; a difference means unsaved edits. */
  snapshot: string;
  /** ISO time of the last successful test, or null when not tested yet. */
  testedAt: string | null;
  password: PasswordKind;
}

const snapshotOf = (p: ConnectionProfile) => JSON.stringify(p);

function passwordText(kind: PasswordKind): string {
  if (kind === 'keychain') return 'Password is in your system keychain.';
  if (kind === 'session') return 'Password is kept in memory only until you close the app.';
  return 'No password is saved. Type it to test again.';
}

/** The "this connection is saved" card, with Retry, Edit and Delete. */
function SavedCard({
  title,
  profile,
  info,
  busy,
  onRetry,
  onEdit,
  onDelete,
}: {
  title: string;
  profile: ConnectionProfile;
  info: SavedInfo;
  busy: boolean;
  onRetry: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const changed = snapshotOf(profile) !== info.snapshot;
  return (
    <div
      className="space-y-2 rounded-xl border border-accent-200 bg-accent-50/60 p-3.5 text-sm dark:border-accent-500/30 dark:bg-accent-500/[0.08]"
      data-testid={`${title}-saved-card`}
    >
      <div className="flex items-center gap-2 font-semibold text-accent-900 dark:text-accent-100" role="status">
        {changed ? (
          <>
            <Pencil className="h-4 w-4" aria-hidden="true" />
            Saved connection, with unsaved changes
          </>
        ) : (
          <>
            <BookmarkCheck className="h-4 w-4" aria-hidden="true" />
            Saved
          </>
        )}
      </div>
      <p className="text-slate-700 dark:text-slate-300">
        {changed
          ? 'Press Save, or test the connection, to keep your changes.'
          : info.testedAt
            ? `Last tested ${formatWhen(info.testedAt)}.`
            : 'Not tested yet in this session.'}
      </p>
      <p className="text-slate-700 dark:text-slate-300">{passwordText(info.password)}</p>
      {!confirming ? (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="secondary" disabled={busy} onClick={onRetry}>
            Retry connection
          </Button>
          <Button size="sm" variant="secondary" onClick={onEdit}>
            Edit
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setConfirming(true)}>
            Delete connection
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Confirm deleting the connection">
          <span className="font-medium text-red-800 dark:text-red-300">
            Delete "{profile.label}" and its saved password?
          </span>
          <Button size="sm" variant="danger" onClick={onDelete}>
            Yes, delete it
          </Button>
          <Button size="sm" variant="secondary" onClick={() => setConfirming(false)}>
            Keep it
          </Button>
        </div>
      )}
    </div>
  );
}

function ProfileEditor({
  title,
  profile,
  onChange,
  password,
  onPasswordChange,
}: {
  title: string;
  profile: ConnectionProfile;
  onChange: (p: ConnectionProfile) => void;
  password: string;
  onPasswordChange: (value: string) => void;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3 pb-1">
        <span className="flex h-9 w-9 items-center justify-center rounded-[10px] bg-gradient-to-b from-accent-500 to-accent-700 text-white shadow-[0_2px_6px_rgba(42,36,135,0.3)]">
          <Database className="h-[18px] w-[18px]" aria-hidden="true" />
        </span>
        <div>
          <h3 className="text-[17px] font-semibold tracking-[-0.01em] text-ink dark:text-white">
            {title}
          </h3>
          <p className="text-xs text-ink-secondary dark:text-slate-400">
            {title === 'Old database' ? 'Before the migration' : 'After the migration'}
          </p>
        </div>
      </div>
      <Input
        label="Label"
        value={profile.label}
        onChange={(e) => onChange({ ...profile, label: e.target.value })}
        data-testid={`${title}-label`}
      />
      <div className="grid grid-cols-2 gap-3">
        <Input
          label="Host"
          value={profile.host}
          onChange={(e) => onChange({ ...profile, host: e.target.value })}
          data-testid={`${title}-host`}
        />
        <Input
          label="Port"
          type="number"
          value={profile.port}
          onChange={(e) => onChange({ ...profile, port: Number(e.target.value) })}
          data-testid={`${title}-port`}
        />
      </div>
      <Input
        label="Database"
        value={profile.database}
        onChange={(e) => onChange({ ...profile, database: e.target.value })}
        data-testid={`${title}-database`}
      />
      <Input
        label="User"
        value={profile.user}
        onChange={(e) => onChange({ ...profile, user: e.target.value })}
        data-testid={`${title}-user`}
      />
      <Input
        label="Password (stored in your system keychain)"
        type="password"
        autoComplete="new-password"
        value={password}
        onChange={(e) => onPasswordChange(e.target.value)}
        data-testid={`${title}-password`}
      />
      <Select
        label="Encryption"
        options={encryptionOptions}
        value={profile.encryption}
        onChange={(e) =>
          onChange({ ...profile, encryption: e.target.value as EncryptionMode })
        }
        data-testid={`${title}-encryption`}
      />
      {profile.encryption === 'verify' && (
        <div className="space-y-1">
          <Input
            label="Your bank's certificate authority file (optional)"
            value={profile.caFile ?? ''}
            onChange={(e) => onChange({ ...profile, caFile: e.target.value })}
            data-testid={`${title}-ca-file`}
          />
          <p className="text-xs text-slate-500 dark:text-slate-400">
            A PEM bundle of your bank's private certificate authority, so an internal server can
            be verified.
          </p>
        </div>
      )}
      {profile.encryption === 'trust' && (
        <div className="space-y-1">
          <Input
            label="Certificate fingerprint (SHA-256)"
            value={profile.tlsFingerprint ?? ''}
            onChange={(e) => onChange({ ...profile, tlsFingerprint: e.target.value })}
            data-testid={`${title}-fingerprint`}
          />
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Trust this certificate once. Shown when the server's certificate cannot be verified.
          </p>
        </div>
      )}
      {profile.encryption === 'allow-plaintext' && (
        <div className="space-y-1">
          <Input
            label="Reason for unencrypted connection"
            value={profile.plaintextReason ?? ''}
            onChange={(e) => onChange({ ...profile, plaintextReason: e.target.value })}
            data-testid={`${title}-plaintext-reason`}
          />
          <p className="text-xs text-slate-500 dark:text-slate-400">
            Required. Your choice to connect without encryption, and this reason, are written
            into the report.
          </p>
        </div>
      )}
    </div>
  );
}

function ServerInfoCard({ info }: { info: ServerInfo }) {
  const encryption =
    info.tls.state === 'encrypted' ? (
      info.tls.verified ? (
        <span className="inline-flex items-center gap-1 text-emerald-700 dark:text-emerald-300">
          <ShieldCheck className="h-4 w-4" />
          Encrypted and verified
        </span>
      ) : (
        <span className="inline-flex items-center gap-1 text-amber-700 dark:text-amber-300">
          <Shield className="h-4 w-4" />
          Encrypted, unverified fingerprint {info.tls.fingerprint}
        </span>
      )
    ) : (
      <span className="inline-flex items-center gap-1 text-red-700 dark:text-red-300">
        <ShieldAlert className="h-4 w-4" />
        Not encrypted
      </span>
    );

  return (
    <div className="animate-scale-in space-y-2 rounded-xl border border-emerald-200/80 bg-emerald-50/50 p-3.5 text-sm dark:border-emerald-500/25 dark:bg-emerald-500/[0.07]">
      <div
        role="status"
        className="flex items-center gap-2 text-base font-semibold text-emerald-800 dark:text-emerald-300"
      >
        <CheckCircle className="h-5 w-5 animate-pop" aria-hidden="true" />
        Connected
      </div>
      <p className="text-xs text-slate-600 dark:text-slate-400">
        Dmux checked the server version, the text encoding, the encryption, and whether this login
        is read-only.
      </p>
      <div className="flex items-center gap-2">
        <span className="w-32 shrink-0 text-ink-secondary dark:text-slate-400">Server version:</span> {info.version}
      </div>
      <div className="flex items-center gap-2">
        <span className="w-32 shrink-0 text-ink-secondary dark:text-slate-400">Encoding:</span> {info.encoding}
      </div>
      <div className="flex items-center gap-2">
        <span className="w-32 shrink-0 text-ink-secondary dark:text-slate-400">Collation:</span> {info.collation}
      </div>
      <div className="flex items-center gap-2">
        <span className="w-32 shrink-0 text-ink-secondary dark:text-slate-400">Encryption:</span> {encryption}
      </div>
      {!info.canWrite && !info.isSuperuser && (
        <div className="flex items-center gap-2">
          <span className="w-32 shrink-0 text-ink-secondary dark:text-slate-400">Read-only login:</span> yes
        </div>
      )}
      {info.isSuperuser && (
        <div className="flex items-center gap-2">
          <StatusBadge status="WARNING" />
          <Tooltip
            label="Superuser"
            explanation="A superuser login can run any command, even inside a read-only transaction. Dmux refuses superuser logins by default."
          >
            Superuser login
          </Tooltip>
          (allowed by you)
        </div>
      )}
      {info.canWrite && (
        <div className="flex items-center gap-2">
          <StatusBadge status="WARNING" />
          This login can write data.
        </div>
      )}
      {info.poolerSuspected && (
        <div className="flex items-center gap-2">
          <StatusBadge status="WARNING" />
          A connection pooler may be present.
        </div>
      )}
      {!info.md5Available && (
        <div className="flex items-center gap-2">
          <StatusBadge status="WARNING" />
          md5 hashing is not available; Dmux will hash in the app.
        </div>
      )}
      {info.warning && <p className="text-amber-800 dark:text-amber-200">{info.warning}</p>}
      {info.sessionPassword && (
        <p className="text-slate-700 dark:text-slate-300" data-testid="session-password-notice">
          {SESSION_PASSWORD_NOTICE}
        </p>
      )}
    </div>
  );
}

/**
 * The only way to use a superuser login: the user types why, and the reason is
 * recorded in the results (PLAN 6.2).  Without a reason nothing is allowed.
 */
function SuperuserOverridePanel({
  allowedReason,
  onAllow,
  onRevoke,
}: {
  allowedReason: string | null;
  onAllow: (reason: string) => void;
  onRevoke: () => void;
}) {
  const [reason, setReason] = useState('');
  if (allowedReason) {
    return (
      <Panel>
        <div className="space-y-2 text-sm" data-testid="superuser-override">
          <div className="flex items-center gap-2">
            <StatusBadge status="WARNING" />
            <span className="font-medium">Superuser login allowed.</span>
          </div>
          <p className="text-slate-700 dark:text-slate-300">
            Your reason is recorded in the results and the report: “{allowedReason}”
          </p>
          <Button variant="secondary" size="sm" onClick={onRevoke}>
            Stop allowing superuser
          </Button>
        </div>
      </Panel>
    );
  }
  return (
    <Panel>
      <div className="space-y-3 text-sm" data-testid="superuser-override">
        <h3 className="text-base font-semibold text-slate-900 dark:text-slate-100">
          This login is a superuser
        </h3>
        <p className="text-slate-700 dark:text-slate-300">
          A superuser can change or delete anything, even while Dmux only reads. Ask your database
          administrator for a read-only login. If you must use this one, type why. The reason is
          written into the results and the report.
        </p>
        <Input
          label="Reason for using a superuser login (required)"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          data-testid="superuser-reason"
        />
        <Button
          variant="danger"
          size="sm"
          disabled={!reason.trim()}
          onClick={() => onAllow(reason.trim())}
        >
          Allow superuser and test again
        </Button>
      </div>
    </Panel>
  );
}

export function ConnectScreen({
  engine,
  oldProfile,
  newProfile,
  onChangeOld,
  onChangeNew,
  onNext,
}: ConnectScreenProps) {
  const [oldResult, setOldResult] = useState<Awaited<ReturnType<Engine['testConnection']>> | null>(
    null,
  );
  const [newResult, setNewResult] = useState<Awaited<ReturnType<Engine['testConnection']>> | null>(
    null,
  );
  const [testing, setTesting] = useState<'old' | 'new' | null>(null);
  const [saving, setSaving] = useState<'old' | 'new' | null>(null);
  // Bumped after every save so the saved-connection lists refresh.
  const [savedVersion, setSavedVersion] = useState(0);
  const [savedInfo, setSavedInfo] = useState<{ old: SavedInfo | null; new: SavedInfo | null }>({
    old: null,
    new: null,
  });
  const [testNotice, setTestNotice] = useState<{ old: string | null; new: string | null }>({
    old: null,
    new: null,
  });

  function markPicked(side: 'old' | 'new', p: ConnectionProfile, has: boolean) {
    setTestNotice((n) => ({ ...n, [side]: null }));
    setSavedInfo((i) => ({
      ...i,
      [side]: { snapshot: snapshotOf(p), testedAt: null, password: has ? 'keychain' : 'none' },
    }));
  }

  // The typed password lives only in this component's local state, is sent once
  // to the backend on Save/Test, and is cleared as soon as the call returns —
  // never lifted into app state, never persisted, never logged (PLAN 6.5).
  const [oldPassword, setOldPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [oldPasswordSaved, setOldPasswordSaved] = useState(false);
  const [newPasswordSaved, setNewPasswordSaved] = useState(false);
  const [oldSaveWarning, setOldSaveWarning] = useState<string | null>(null);
  const [newSaveWarning, setNewSaveWarning] = useState<string | null>(null);
  const [oldSaveError, setOldSaveError] = useState<string | null>(null);
  const [newSaveError, setNewSaveError] = useState<string | null>(null);

  const bothOk = oldResult?.ok && newResult?.ok;
  const [superuserReason, setSuperuserReason] = useState<string | null>(null);
  const superuserSeen =
    Boolean(oldResult?.superuserRefused || newResult?.superuserRefused) ||
    Boolean(oldResult?.info?.isSuperuser || newResult?.info?.isSuperuser);

  async function retestAfterOverride() {
    // Re-test the sides the refusal hit; the backend reads the saved password.
    if (oldResult?.superuserRefused) {
      setOldResult(await engine.testConnection(oldProfile));
    }
    if (newResult?.superuserRefused) {
      setNewResult(await engine.testConnection(newProfile));
    }
  }

  async function allowSuperuser(reason: string) {
    engine.setSuperuserOverride({ reason });
    setSuperuserReason(reason);
    await retestAfterOverride();
  }

  function revokeSuperuser() {
    engine.setSuperuserOverride(null);
    setSuperuserReason(null);
    // A superuser result is no longer valid without the override.
    if (oldResult?.info?.isSuperuser) setOldResult(null);
    if (newResult?.info?.isSuperuser) setNewResult(null);
  }

  function describeThrown(error: unknown): string {
    return describeError(error);
  }

  async function saveOld() {
    setSaving('old');
    setOldSaveError(null);
    setOldSaveWarning(null);
    try {
      const result = await engine.saveProfile(oldProfile, oldPassword || undefined);
      setOldSaveWarning(result.warning ?? null);
      setOldPasswordSaved(await engine.profileHasPassword(oldProfile.id));
      onChangeOld(result.profile);
      const hasOld = await engine.profileHasPassword(result.profile.id);
      const kindOld: PasswordKind = result.warning?.includes('memory') ? 'session' : hasOld ? 'keychain' : 'none';
      setSavedInfo((i) => ({ ...i, old: { snapshot: snapshotOf(result.profile), testedAt: i.old?.testedAt ?? null, password: kindOld } }));
      setSavedVersion((v) => v + 1);
    } catch (error) {
      setOldSaveError(describeThrown(error));
    } finally {
      setOldPassword('');
      setSaving(null);
    }
  }

  async function saveNew() {
    setSaving('new');
    setNewSaveError(null);
    setNewSaveWarning(null);
    try {
      const result = await engine.saveProfile(newProfile, newPassword || undefined);
      setNewSaveWarning(result.warning ?? null);
      setNewPasswordSaved(await engine.profileHasPassword(newProfile.id));
      onChangeNew(result.profile);
      const hasNew = await engine.profileHasPassword(result.profile.id);
      const kindNew: PasswordKind = result.warning?.includes('memory') ? 'session' : hasNew ? 'keychain' : 'none';
      setSavedInfo((i) => ({ ...i, new: { snapshot: snapshotOf(result.profile), testedAt: i.new?.testedAt ?? null, password: kindNew } }));
      setSavedVersion((v) => v + 1);
    } catch (error) {
      setNewSaveError(describeThrown(error));
    } finally {
      setNewPassword('');
      setSaving(null);
    }
  }

  /**
   * Test one side.  A successful test also saves the connection (the password,
   * if typed, goes to the keychain or the in-memory session cache, never to
   * SQLite), so it is kept for next time.
   */
  async function runTest(side: 'old' | 'new') {
    const profile = side === 'old' ? oldProfile : newProfile;
    const typed = side === 'old' ? oldPassword : newPassword;
    const setResult = side === 'old' ? setOldResult : setNewResult;
    const setPassword = side === 'old' ? setOldPassword : setNewPassword;
    const setPasswordSaved = side === 'old' ? setOldPasswordSaved : setNewPasswordSaved;
    const setWarning = side === 'old' ? setOldSaveWarning : setNewSaveWarning;
    const onChange = side === 'old' ? onChangeOld : onChangeNew;
    setTesting(side);
    setTestNotice((n) => ({ ...n, [side]: null }));
    try {
      // With no typed password the backend tests the SAVED details.  If the
      // form differs from the saved ones, say so instead of testing the wrong
      // server.
      if (!typed) {
        const existing = (await engine.listProfiles()).find((p) => p.id === profile.id);
        if (existing && snapshotOf(existing) !== snapshotOf(profile)) {
          setResult({
            ok: false,
            error:
              'You changed the connection details. Type the password to test them, or press Edit to go back to the saved details.',
          });
          return;
        }
      }
      const result = await engine.testConnection(profile, typed || undefined);
      setResult(result);
      if (!result.ok) return;
      try {
        const saved = await engine.saveProfile(profile, typed || undefined);
        setWarning(saved.warning ?? null);
        const has = await engine.profileHasPassword(saved.profile.id);
        const kind: PasswordKind =
          result.info?.sessionPassword || saved.warning?.includes('memory')
            ? 'session'
            : has
              ? 'keychain'
              : 'none';
        setPasswordSaved(kind === 'keychain');
        onChange(saved.profile);
        setSavedInfo((i) => ({
          ...i,
          [side]: {
            snapshot: snapshotOf(saved.profile),
            testedAt: new Date().toISOString(),
            password: kind,
          },
        }));
        setSavedVersion((v) => v + 1);
      } catch (error) {
        setTestNotice((n) => ({
          ...n,
          [side]: `Connected, but the connection could not be saved: ${describeThrown(error)}`,
        }));
      }
    } finally {
      setPassword('');
      setTesting(null);
    }
  }

  const testOld = () => runTest('old');
  const testNew = () => runTest('new');

  async function deleteSaved(side: 'old' | 'new') {
    const profile = side === 'old' ? oldProfile : newProfile;
    try {
      await engine.deleteProfile(profile.id);
      setSavedInfo((i) => ({ ...i, [side]: null }));
      (side === 'old' ? setOldResult : setNewResult)(null);
      (side === 'old' ? setOldPasswordSaved : setNewPasswordSaved)(false);
      setTestNotice((n) => ({ ...n, [side]: `Deleted the saved connection "${profile.label}" and its saved password.` }));
      setSavedVersion((v) => v + 1);
    } catch (error) {
      setTestNotice((n) => ({ ...n, [side]: describeThrown(error) }));
    }
  }

  function editSaved(side: 'old' | 'new') {
    const title = side === 'old' ? 'Old database' : 'New database';
    document.querySelector<HTMLInputElement>(`[data-testid="${title}-host"]`)?.focus();
  }

  function savedCard(side: 'old' | 'new') {
    const info = savedInfo[side];
    const title = side === 'old' ? 'Old database' : 'New database';
    return (
      <>
        {testNotice[side] && (
          <p className="text-sm text-slate-700 dark:text-slate-300" role="status">
            {testNotice[side]}
          </p>
        )}
        {info && (
          <SavedCard
            title={title}
            profile={side === 'old' ? oldProfile : newProfile}
            info={info}
            busy={testing !== null || saving !== null}
            onRetry={() => void runTest(side)}
            onEdit={() => editSaved(side)}
            onDelete={() => void deleteSaved(side)}
          />
        )}
      </>
    );
  }

  return (
    <div className="space-y-6">
      <ScreenHeader title="Connect" subtitle="Enter the old and new databases. Dmux only ever reads them." />
      <div className="grid gap-6 lg:grid-cols-2">
        <Panel>
          <SavedProfilePicker
            engine={engine}
            title="Old database"
            otherProfileId={newProfile.id}
            reloadKey={savedVersion}
            onPick={(p, has) => {
              onChangeOld(p);
              setOldPassword('');
              setOldResult(null);
              setOldPasswordSaved(false);
              markPicked('old', p, has);
            }}
          />
          <ProfileEditor
            title="Old database"
            profile={oldProfile}
            onChange={onChangeOld}
            password={oldPassword}
            onPasswordChange={setOldPassword}
          />
          <div className="mt-4 space-y-3">
            <div className="flex gap-2">
              <Button
                onClick={saveOld}
                disabled={saving === 'old' || testing === 'old'}
                variant="secondary"
                className="flex-1"
              >
                {saving === 'old' ? 'Saving…' : 'Save'}
              </Button>
              <Button
                onClick={testOld}
                disabled={saving === 'old' || testing === 'old'}
                variant="secondary"
                className="flex-1"
              >
                {testing === 'old' ? 'Testing…' : 'Test connection'}
              </Button>
            </div>
            {oldPasswordSaved && (
              <p className="text-sm text-emerald-700 dark:text-emerald-300">
                Password saved in keychain
              </p>
            )}
            {oldSaveWarning && (
              <p className="text-sm text-amber-700 dark:text-amber-300">{oldSaveWarning}</p>
            )}
            {oldSaveError && (
              <ErrorCard title="Could not save profile" className="mt-3">
                {oldSaveError}
              </ErrorCard>
            )}
            {oldResult && !oldResult.ok && (
              <ErrorCard title="Connection failed" className="mt-3">
                {oldResult.error}
              </ErrorCard>
            )}
            {oldResult?.ok && oldResult.info && <ServerInfoCard info={oldResult.info} />}
            {savedCard('old')}
          </div>
        </Panel>
        <Panel>
          <SavedProfilePicker
            engine={engine}
            title="New database"
            otherProfileId={oldProfile.id}
            reloadKey={savedVersion}
            onPick={(p, has) => {
              onChangeNew(p);
              setNewPassword('');
              setNewResult(null);
              setNewPasswordSaved(false);
              markPicked('new', p, has);
            }}
          />
          <ProfileEditor
            title="New database"
            profile={newProfile}
            onChange={onChangeNew}
            password={newPassword}
            onPasswordChange={setNewPassword}
          />
          <div className="mt-4 space-y-3">
            <div className="flex gap-2">
              <Button
                onClick={saveNew}
                disabled={saving === 'new' || testing === 'new'}
                variant="secondary"
                className="flex-1"
              >
                {saving === 'new' ? 'Saving…' : 'Save'}
              </Button>
              <Button
                onClick={testNew}
                disabled={saving === 'new' || testing === 'new'}
                variant="secondary"
                className="flex-1"
              >
                {testing === 'new' ? 'Testing…' : 'Test connection'}
              </Button>
            </div>
            {newPasswordSaved && (
              <p className="text-sm text-emerald-700 dark:text-emerald-300">
                Password saved in keychain
              </p>
            )}
            {newSaveWarning && (
              <p className="text-sm text-amber-700 dark:text-amber-300">{newSaveWarning}</p>
            )}
            {newSaveError && (
              <ErrorCard title="Could not save profile" className="mt-3">
                {newSaveError}
              </ErrorCard>
            )}
            {newResult && !newResult.ok && (
              <ErrorCard title="Connection failed" className="mt-3">
                {newResult.error}
              </ErrorCard>
            )}
            {newResult?.ok && newResult.info && <ServerInfoCard info={newResult.info} />}
            {savedCard('new')}
          </div>
        </Panel>
      </div>
      {superuserSeen && (
        <SuperuserOverridePanel
          allowedReason={superuserReason}
          onAllow={(reason) => void allowSuperuser(reason)}
          onRevoke={revokeSuperuser}
        />
      )}
      <div className="flex flex-wrap items-center justify-end gap-4">
        <p className="text-sm text-slate-700 dark:text-slate-300" role="status">
          {bothOk
            ? 'Both databases are connected. Next, compare their structure.'
            : 'Test both connections to continue.'}
        </p>
        <Button onClick={onNext} disabled={!bothOk} variant="primary" size="lg">
          Compare structure
        </Button>
      </div>
    </div>
  );
}
