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
  const base = side === 'old' ? '5433' : '5434';
  return {
    id: `${side}-profile`,
    label: `${side === 'old' ? 'Old' : 'New'} database`,
    host: '127.0.0.1',
    port: Number(base),
    database: 'demo_bank',
    user: 'dmux_dev',
    encryption: 'verify',
  };
}

export const defaultOldProfile = () => emptyProfile('old');
export const defaultNewProfile = () => emptyProfile('new');

function ProfileEditor({
  title,
  profile,
  onChange,
}: {
  title: string;
  profile: ConnectionProfile;
  onChange: (p: ConnectionProfile) => void;
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
      <Select
        label="Encryption"
        options={encryptionOptions}
        value={profile.encryption}
        onChange={(e) =>
          onChange({ ...profile, encryption: e.target.value as EncryptionMode })
        }
        data-testid={`${title}-encryption`}
      />
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
  const [loading, setLoading] = useState<'old' | 'new' | null>(null);

  const bothOk = oldResult?.ok && newResult?.ok;

  async function testOld() {
    setLoading('old');
    const result = await engine.testConnection(oldProfile);
    setOldResult(result);
    setLoading(null);
  }

  async function testNew() {
    setLoading('new');
    const result = await engine.testConnection(newProfile);
    setNewResult(result);
    setLoading(null);
  }

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-semibold text-slate-900 dark:text-slate-100">Connect</h2>
      <div className="grid gap-6 lg:grid-cols-2">
        <Panel>
          <ProfileEditor title="Old database" profile={oldProfile} onChange={onChangeOld} />
          <div className="mt-4">
            <Button
              onClick={testOld}
              disabled={loading === 'old'}
              variant="secondary"
              className="w-full"
            >
              {loading === 'old' ? 'Testing…' : 'Test connection'}
            </Button>
            {oldResult && !oldResult.ok && (
              <ErrorCard title="Connection failed" className="mt-3">
                {oldResult.error}
              </ErrorCard>
            )}
            {oldResult?.ok && oldResult.info && <ServerInfoCard info={oldResult.info} />}
          </div>
        </Panel>
        <Panel>
          <ProfileEditor title="New database" profile={newProfile} onChange={onChangeNew} />
          <div className="mt-4">
            <Button
              onClick={testNew}
              disabled={loading === 'new'}
              variant="secondary"
              className="w-full"
            >
              {loading === 'new' ? 'Testing…' : 'Test connection'}
            </Button>
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
