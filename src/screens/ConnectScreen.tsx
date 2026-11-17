import { useState } from 'react';
import { Panel } from '../design/Panel';
import { Button } from '../design/Button';
import { Input } from '../design/Input';
import { Select } from '../design/Select';
import { ErrorCard } from '../design/ErrorCard';
import { StatusBadge } from '../design/StatusBadge';
import { Tooltip } from '../design/Tooltip';
import type { Engine } from '../api/engine';
import type { ConnectionProfile, EncryptionMode, ServerInfo } from '../api/contract';
import { Shield, ShieldAlert, ShieldCheck } from 'lucide-react';

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
      <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">
        {title}
      </h3>
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
            Required. This choice is recorded in the report (PLAN 6.7).
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
    <div className="space-y-2 rounded-lg border border-slate-200 bg-white p-3 text-sm dark:border-slate-700 dark:bg-slate-900">
      <div className="flex items-center gap-2">
        <span className="font-medium">Server version:</span> {info.version}
      </div>
      <div className="flex items-center gap-2">
        <span className="font-medium">Encoding:</span> {info.encoding}
      </div>
      <div className="flex items-center gap-2">
        <span className="font-medium">Collation:</span> {info.collation}
      </div>
      <div className="flex items-center gap-2">
        <span className="font-medium">Encryption:</span> {encryption}
      </div>
      {info.isSuperuser && (
        <div className="flex items-center gap-2">
          <StatusBadge status="FAIL" />
          <Tooltip
            label="Superuser"
            explanation="A superuser login can run any command, even inside a read-only transaction. Dmux refuses superuser logins by default."
          >
            Superuser login
          </Tooltip>
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
    </div>
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
      <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">Connect</h2>
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
      <div className="flex justify-end">
        <Button onClick={onNext} disabled={!bothOk} variant="primary" size="lg">
          Compare structure
        </Button>
      </div>
    </div>
  );
}
