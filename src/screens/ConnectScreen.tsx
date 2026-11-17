import { useState } from 'react';
import { Panel } from '../design/Panel';
import { ScreenHeader } from '../design/ScreenHeader';
import { Button } from '../design/Button';
import { Input } from '../design/Input';
import { Select } from '../design/Select';
import { ErrorCard } from '../design/ErrorCard';
import { StatusBadge } from '../design/StatusBadge';
import { Tooltip } from '../design/Tooltip';
import type { Engine } from '../api/engine';
import type { ConnectionProfile, EncryptionMode, ServerInfo } from '../api/contract';
import { CheckCircle, Database, Shield, ShieldAlert, ShieldCheck } from 'lucide-react';

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
    return error instanceof Error ? error.message : String(error);
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
    } catch (error) {
      setNewSaveError(describeThrown(error));
    } finally {
      setNewPassword('');
      setSaving(null);
    }
  }

  async function testOld() {
    setTesting('old');
    try {
      const result = await engine.testConnection(oldProfile, oldPassword || undefined);
      setOldResult(result);
    } finally {
      setOldPassword('');
      setTesting(null);
    }
  }

  async function testNew() {
    setTesting('new');
    try {
      const result = await engine.testConnection(newProfile, newPassword || undefined);
      setNewResult(result);
    } finally {
      setNewPassword('');
      setTesting(null);
    }
  }

  return (
    <div className="space-y-6">
      <ScreenHeader title="Connect" subtitle="Enter the old and new databases. Dmux only ever reads them." />
      <div className="grid gap-6 lg:grid-cols-2">
        <Panel>
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
          </div>
        </Panel>
        <Panel>
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
