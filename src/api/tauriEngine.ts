/**
 * The real engine: the Tauri backend, reached through `@tauri-apps/api` invoke.
 *
 * Structure half (Connect / Compare structure / Map) calls the backend commands
 * and maps their serde (snake_case) JSON onto the frontend contract.  The data
 * half (checks / run / results / report) is still owned by the mock engine
 * until T022 lands — those methods delegate to the deterministic mock.
 *
 * Every mapping is strict: an unknown status or verdict is treated as a
 * non-PASS result, never silently coerced to "same" or "PASS" (a false PASS is
 * the worst bug this project can have).
 */

import { invoke } from '@tauri-apps/api/core';
import { createMockEngine } from './mockEngine';
import type { Engine } from './engine';
import type {
  CheckPlan,
  ConnectionProfile,
  ConnectionTestResult,
  Inventory,
  Mapping,
  MappingMatch,
  IgnoreItem,
  MatchConfidence,
  ReportRequest,
  ReportResult,
  Results,
  RunDefinition,
  RunProgress,
  SchemaDiffNode,
  ServerInfo,
  Verdict,
} from './contract';

// ---------------------------------------------------------------------------
// Engine-native (snake_case) shapes, matching the Rust commands' serde output.
// ---------------------------------------------------------------------------

/** Engine `db::ServerInfo`. */
interface RawServerInfo {
  version_num: number;
  version_string: string;
  encoding: string;
  collation: string;
  ctype: string;
  lc_monetary: string;
  is_superuser: boolean;
  can_write: boolean;
  pooler_suspected: boolean;
  md5_available: boolean;
  sql_ascii: boolean;
}

/** Engine `schema_diff::DiffStatus`. */
type RawDiffStatus = 'equal' | 'different' | 'warning';

/** Engine `schema_diff::SchemaDiffEntry`. */
interface RawSchemaDiffEntry {
  path: string[];
  status: RawDiffStatus;
  old_value: string | null;
  new_value: string | null;
  message: string;
  children: RawSchemaDiffEntry[];
}

/** Engine `schema_diff::SchemaDiff`. */
interface RawSchemaDiff {
  status: RawDiffStatus;
  old_encoding: string;
  new_encoding: string;
  old_collation: string;
  new_collation: string;
  entries: RawSchemaDiffEntry[];
}

/** Engine `mapping::MappingStatus` / rule kinds (snake_case). */
interface RawRule {
  kind: string;
  pattern: string;
  target: string;
}

/** Engine `mapping::Mapping`. */
interface RawMapping {
  version: number;
  rules: RawRule[];
  schema_mappings: Record<string, RawSchemaMapping>;
  ignored: RawIgnoredItem[];
  mapping_hash: string;
}

interface RawSchemaMapping {
  target_schema: string;
  table_mappings: Record<string, RawTableMapping>;
}

interface RawTableMapping {
  target_table: string;
  status: string;
  column_mappings: Record<string, RawColumnMapping>;
}

interface RawColumnMapping {
  target_column: string;
  status: string;
}

interface RawIgnoredItem {
  path: string;
  reason: string;
}

/** The `build_mapping` command's payload. */
interface RawMappingBuild {
  mapping: RawMapping;
  needs_attention: RawAttentionEntry[];
  suggestions: RawSuggestionEntry[];
}

interface RawAttentionEntry {
  path: string;
  issue: string;
  candidates: string[];
}

interface RawSuggestionEntry {
  path: string;
  candidate: string;
  confidence: number;
}

/** The `export_mapping` command's payload. */
interface RawExport {
  json: string;
  hash: string;
}

/** A serde-tagged `CommandError` thrown by the backend. */
interface RawCommandError {
  kind: string;
  message?: string;
  fingerprint?: string;
  reason?: string;
  profile?: string;
}

/** Shape the profile commands expect (the backend never stores a password). */
type EncryptionField = 'verify' | 'trust' | 'allow-plaintext';

interface BackendProfile {
  id: string;
  label: string;
  host: string;
  port: number;
  database: string;
  user: string;
  encryption: EncryptionField;
  tls_fingerprint: string | null;
  plaintext_reason: string | null;
}

// ---------------------------------------------------------------------------
// Strict mapping helpers (exported for unit tests).
// ---------------------------------------------------------------------------

/**
 * Map an engine verdict string onto the contract.  The list is fixed and
 * exhaustive; any value outside it maps to `TOOL_FAULT` (a non-PASS result),
 * never to PASS.
 */
export function mapVerdict(raw: unknown): Verdict {
  const known: Verdict[] = [
    'PASS',
    'FAIL',
    'WIDELY_DIFFERENT',
    'NEEDS_REVIEW',
    'CHANGED_DURING_CHECK',
    'TOOL_FAULT',
    'WARNING',
  ];
  if (typeof raw === 'string' && (known as string[]).includes(raw)) {
    return raw as Verdict;
  }
  return 'TOOL_FAULT';
}

/**
 * Map an engine schema-diff status plus its message onto the contract's
 * per-node status.  1:1, no `same` fallback:
 *
 *   - `equal`                 -> same
 *   - message says "added"    -> added   (green)
 *   - message says "missing"  -> missing  (red)
 *   - `different`             -> changed  (a real difference)
 *   - `warning`               -> changed  (amber)
 *   - anything else           -> changed  (conservative, never "same")
 */
export function mapDiffStatus(
  status: RawDiffStatus,
  message: string,
): SchemaDiffNode['status'] {
  if (status === 'equal') {
    return 'same';
  }
  const text = message.toLowerCase();
  if (text.includes('added')) {
    return 'added';
  }
  if (text.includes('missing')) {
    return 'missing';
  }
  return 'changed';
}

/** Derive a tree-node `kind` from the engine's `path` segments. */
function kindFromPath(path: string[]): SchemaDiffNode['kind'] {
  const objectKinds = new Set<SchemaDiffNode['kind']>([
    'table',
    'column',
    'index',
    'constraint',
    'sequence',
    'view',
    'function',
    'extension',
    'trigger',
  ]);
  const found = path.find((segment) => objectKinds.has(segment as SchemaDiffNode['kind']));
  if (found) {
    return found as SchemaDiffNode['kind'];
  }
  if (path.includes('schema')) {
    return 'schema';
  }
  return 'column';
}

/** Convert the engine's flat diff entries into the contract's nested tree. */
function translateDiff(entries: RawSchemaDiffEntry[]): SchemaDiffNode[] {
  const out: SchemaDiffNode[] = [];
  for (const entry of entries) {
    const path = entry.path;
    if (path.length === 0) {
      continue;
    }
    const node: SchemaDiffNode = {
      id: path.join('/'),
      kind: kindFromPath(path),
      name: path[path.length - 1] ?? path[0],
      status: mapDiffStatus(entry.status, entry.message),
    };
    if (entry.old_value != null) {
      node.old = entry.old_value;
    }
    if (entry.new_value != null) {
      node.new = entry.new_value;
    }
    if (entry.children.length > 0) {
      node.children = translateDiff(entry.children);
    }
    out.push(node);
  }
  return out;
}

/** Convert an engine mapping status to the contract `MatchConfidence`. */
function mapConfidence(status: string): MatchConfidence {
  switch (status) {
    case 'exact':
      return 'exact';
    case 'case_insensitive':
      return 'case';
    case 'fuzzy':
      return 'fuzzy';
    default:
      return 'fuzzy';
  }
}

/** Project the engine `Mapping` + attention queue onto the contract `Mapping`. */
function translateMapping(
  build: RawMappingBuild,
  inventory: Inventory,
): Mapping {
  const matches: MappingMatch[] = [];
  const needsAttention: MappingMatch[] = [];

  // Approved (exact/case) matches come from the engine's schema/table/column
  // mappings; the attention queue is the human-review items the backend flags.
  for (const [schema, schemaMapping] of Object.entries(build.mapping.schema_mappings)) {
    for (const [oldTable, tableMapping] of Object.entries(schemaMapping.table_mappings)) {
      matches.push({
        id: `${schema}.${oldTable}`,
        oldName: `${schema}.${oldTable}`,
        newName: `${schemaMapping.target_schema}.${tableMapping.target_table}`,
        confidence: mapConfidence(tableMapping.status),
        approved: tableMapping.status !== 'ignored' && tableMapping.status !== 'unmapped',
      });
      for (const [oldCol, colMapping] of Object.entries(tableMapping.column_mappings)) {
        matches.push({
          id: `${schema}.${oldTable}.${oldCol}`,
          oldName: `${oldTable}.${oldCol}`,
          newName: colMapping.target_column,
          confidence: mapConfidence(colMapping.status),
          approved: colMapping.status !== 'ignored' && colMapping.status !== 'unmapped',
        });
      }
    }
  }

  for (const attention of build.needs_attention) {
    needsAttention.push({
      id: attention.path,
      oldName: attention.path,
      newName: attention.candidates[0] ?? '',
      confidence: 'fuzzy',
      approved: false,
    });
  }

  const ignored: IgnoreItem[] = build.mapping.ignored.map((item) => ({
    id: item.path,
    name: item.path,
    reason: item.reason,
  }));

  const rules: string[] = build.mapping.rules.map(
    (rule) => `${rule.kind}: ${rule.pattern} -> ${rule.target}`,
  );

  // Surface the encoding/collation differences as review items, matching the
  // mock's behaviour so the Map screen always surfaces them.
  if (inventory.oldEncoding !== inventory.newEncoding) {
    needsAttention.push({
      id: 'encoding',
      oldName: `encoding ${inventory.oldEncoding}`,
      newName: `encoding ${inventory.newEncoding}`,
      confidence: 'fuzzy',
      approved: false,
    });
  }

  return { rules, matches, needsAttention, ignored };
}

// ---------------------------------------------------------------------------
// Profile / invoke adapters.
// ---------------------------------------------------------------------------

function toBackendProfile(profile: ConnectionProfile): BackendProfile {
  // The frontend contract carries the encryption mode but not a trust
  // fingerprint or a plaintext reason (that is a T023-follow-up gap).  For now
  // `trust` and `allow-plaintext` are passed with null extras and the backend
  // rejects them with a clear, typed error rather than guessing.
  const encryption: EncryptionField =
    profile.encryption === 'trust' ? 'trust' : profile.encryption === 'allow-plaintext' ? 'allow-plaintext' : 'verify';
  return {
    id: profile.id,
    label: profile.label,
    host: profile.host,
    port: profile.port,
    database: profile.database,
    user: profile.user,
    encryption,
    tls_fingerprint: null,
    plaintext_reason: null,
  };
}

function toConnectInput(profile: ConnectionProfile): {
  profile: BackendProfile;
  password: null;
} {
  // The password is read from the keychain on the backend; the UI never holds
  // it (PLAN 6.5).
  return { profile: toBackendProfile(profile), password: null };
}

function toServerInfo(raw: RawServerInfo): ServerInfo {
  return {
    version: raw.version_string,
    encoding: raw.encoding,
    collation: raw.collation,
    isSuperuser: raw.is_superuser,
    canWrite: raw.can_write,
    poolerSuspected: raw.pooler_suspected,
    md5Available: raw.md5_available,
    tls: { state: 'encrypted', verified: true },
  };
}

/** Turn a thrown (serde-tagged) backend error into a readable string. */
function describeError(error: unknown): string {
  const e = error as Partial<RawCommandError>;
  if (e && typeof e === 'object' && typeof e.kind === 'string') {
    if (e.kind === 'untrusted_cert' && typeof e.fingerprint === 'string') {
      return `server certificate is not trusted; fingerprint=${e.fingerprint}`;
    }
    if (e.kind === 'superuser_refused') {
      return 'superuser login refused by Dmux';
    }
    if (e.kind === 'plaintext_reason_required') {
      return 'a reason is required for an unencrypted connection';
    }
    if (typeof e.message === 'string') {
      return e.message;
    }
    return e.kind;
  }
  return error instanceof Error ? error.message : String(error);
}

// ---------------------------------------------------------------------------
// The engine.
// ---------------------------------------------------------------------------

export interface TauriEngineOptions {
  /**
   * The mock engine the data half (checks / run / results / report) delegates
   * to until T022 wires those to the backend.  Defaults to the deterministic
   * mock; tests inject their own or force a fixture.
   */
  dataMock?: Engine;
}

/**
 * Create the Tauri-backed engine.
 *
 * Structure methods call the backend; data methods delegate to `dataMock`.
 *
 * The flow is linear (connect -> compare -> map), so the adapter remembers the
 * profiles used by `compareStructure` and reuses them in `buildMapping`, which
 * the backend computes against the two live databases.  The raw engine mapping
 * produced by `build_mapping` is kept for `saveMapping`.
 */
export function createTauriEngine(options: TauriEngineOptions = {}): Engine {
  const dataMock = options.dataMock ?? createMockEngine();

  let lastOldProfile: ConnectionProfile | null = null;
  let lastNewProfile: ConnectionProfile | null = null;
  let lastRawMapping: RawMapping | null = null;

  return {
    async testConnection(profile: ConnectionProfile): Promise<ConnectionTestResult> {
      try {
        const info = await invoke<RawServerInfo>('probe_connection', {
          input: toConnectInput(profile),
        });
        const serverInfo = toServerInfo(info);
        if (serverInfo.isSuperuser) {
          serverInfo.warning =
            'This login is a superuser. Dmux refuses superuser logins by default.';
        }
        return { ok: true, info: serverInfo };
      } catch (error) {
        return { ok: false, error: describeError(error) };
      }
    },

    async compareStructure(
      oldProfile: ConnectionProfile,
      newProfile: ConnectionProfile,
    ): Promise<Inventory> {
      lastOldProfile = oldProfile;
      lastNewProfile = newProfile;

      const oldInventory = await invoke<Record<string, unknown>>('run_inventory', {
        input: toConnectInput(oldProfile),
      });
      const newInventory = await invoke<Record<string, unknown>>('run_inventory', {
        input: toConnectInput(newProfile),
      });
      const diff = await invoke<RawSchemaDiff>('schema_diff', {
        old: oldInventory,
        new: newInventory,
      });

      return {
        schemas: translateDiff(diff.entries),
        oldEncoding: diff.old_encoding,
        newEncoding: diff.new_encoding,
        oldCollation: diff.old_collation,
        newCollation: diff.new_collation,
      };
    },

    async buildMapping(inventory: Inventory): Promise<Mapping> {
      if (!lastOldProfile || !lastNewProfile) {
        throw new Error('compareStructure must run before buildMapping');
      }
      const build = await invoke<RawMappingBuild>('build_mapping', {
        old: toConnectInput(lastOldProfile),
        new: toConnectInput(lastNewProfile),
      });
      lastRawMapping = build.mapping;
      return translateMapping(build, inventory);
    },

    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the projected mapping is rebuilt from the backend `lastRawMapping`
    async saveMapping(_mapping: Mapping): Promise<{ hash: string; json: string }> {
      if (!lastRawMapping) {
        throw new Error('buildMapping must run before saveMapping');
      }
      const exported = await invoke<RawExport>('export_mapping', {
        mapping: lastRawMapping,
      });
      return { hash: exported.hash, json: exported.json };
    },

    // --- Data half: delegated to the mock until T022. ---

    buildCheckPlan(mapping: Mapping): Promise<CheckPlan> {
      return dataMock.buildCheckPlan(mapping);
    },
    startRun(definition: RunDefinition): Promise<RunProgress> {
      return dataMock.startRun(definition);
    },
    pollRun(runId: string): Promise<RunProgress> {
      return dataMock.pollRun(runId);
    },
    setRunPaused(runId: string, paused: boolean): Promise<RunProgress> {
      return dataMock.setRunPaused(runId, paused);
    },
    cancelRun(runId: string): Promise<RunProgress> {
      return dataMock.cancelRun(runId);
    },
    getResults(runId: string): Promise<Results> {
      return dataMock.getResults(runId);
    },
    exportReport(request: ReportRequest): Promise<ReportResult> {
      return dataMock.exportReport(request);
    },
    async getVersion(): Promise<string> {
      return invoke<string>('get_version');
    },
  };
}