/**
 * The real engine: the Tauri backend, reached through `@tauri-apps/api` invoke.
 *
 * Structure half (Connect / Compare structure / Map) calls the backend commands
 * and maps their serde (snake_case) JSON onto the frontend contract.  The data
 * half (checks / run / results) also calls the backend; each method maps the
 * engine's serde output strictly.  Report export is still a typed `EngineError`
 * (NOT_AVAILABLE) until T024.  A false PASS is the worst bug this project can
 * have, so nothing here ever invents one.
 *
 * Every mapping is strict: an unknown status or verdict is treated as a
 * non-PASS result, never silently coerced to "same" or "PASS".
 */

import { invoke } from '@tauri-apps/api/core';
import type { Engine } from './engine';
import type {
  CheckLevel,
  CheckPlan,
  ConnectionProfile,
  ConnectionTestResult,
  CoverageSummary,
  DrillDownRow,
  FailureCard,
  Inventory,
  Mapping,
  MappingMatch,
  IgnoreItem,
  MatchConfidence,
  ReportResult,
  Results,
  RunDefinition,
  RunProgress,
  SaveProfileResult,
  SchemaDiffNode,
  ServerInfo,
  TableCheck,
  TableProgress,
  TableResult,
  TableRunState,
  TlsState,
  Verdict,
} from './contract';

/** A typed engine error the UI can distinguish from a connection/database error. */
export class EngineError extends Error {
  constructor(
    readonly code: 'NOT_AVAILABLE',
    message: string,
  ) {
    super(message);
    this.name = 'EngineError';
  }
}

/**
 * The engine run stopped with an error (backend `overall_state: "failed"`).
 * `kind` is the backend's typed error kind; the run has no results.  It is
 * thrown from every progress call so the UI shows the error, never "running".
 */
export class RunFailedError extends Error {
  constructor(
    readonly kind: string,
    message: string,
  ) {
    super(message);
    this.name = 'RunFailedError';
  }
}

const NOT_AVAILABLE_MESSAGE =
  'Running checks and exporting reports is not available in this version of Dmux yet.';

function notAvailable(): Promise<never> {
  return Promise.reject(new EngineError('NOT_AVAILABLE', NOT_AVAILABLE_MESSAGE));
}

// ---------------------------------------------------------------------------
// Engine-native (snake_case) shapes, matching the Rust commands' serde output.
// ---------------------------------------------------------------------------

/** Engine `db::ServerInfo` plus the real TLS state (see `gateway::ProbeResult`). */
interface RawTlsState {
  state: 'encrypted' | 'plaintext';
  verified?: boolean;
  fingerprint?: string | null;
  reason?: string;
}

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
  tls: RawTlsState;
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

/** Engine `mapping::Mapping` rule kinds. */
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
  ca_file: string | null;
}

/** Shape `save_profile` returns: the stored profile plus an optional warning. */
interface RawSaveProfileResult {
  profile: BackendProfile;
  warning: string | null;
}

/** The engine's `CheckLevel` serialises as snake_case (`counts_and_totals`, ...). */
type EngineLevel = 'structure' | 'counts_and_totals' | 'full_values';

/** `build_check_plan` -> `CheckPlanDto`. */
interface RawTableCheck {
  table_id: string;
  schema_name: string;
  table_name: string;
  level: EngineLevel;
  control_total_columns: string[] | null;
}

interface RawCheckPlan {
  tables: RawTableCheck[];
  presets: string[];
  estimate_seconds_min: number;
  estimate_seconds_max: number;
}

/** `start_run` -> `StartRunResult`. */
interface RawStartRun {
  run_id: string;
}

/** `poll_run` -> `RunProgressDto`. */
interface RawTableProgress {
  table_id: string;
  schema_name: string;
  table_name: string;
  state: string;
  progress_percent: number;
  eta_seconds: number | null;
}

interface RawRunError {
  kind: string;
  message?: string;
}

interface RawRunProgress {
  run_id: string;
  overall_state: string;
  /** Present when `overall_state` is `failed`: the typed engine error. */
  error?: RawRunError | null;
  tables: RawTableProgress[];
  eta_seconds_min: number | null;
  eta_seconds_max: number | null;
}

/** `get_results` -> `ResultsDto`. */
interface RawDrillDownRow {
  key: string;
  kind: string;
  differing_columns: string[] | null;
}

interface RawFailureCard {
  id: string;
  table_id: string;
  schema_name: string;
  table_name: string;
  level: EngineLevel;
  verdict: string;
  summary: string;
  detail: string;
}

interface RawTableResult {
  table_id: string;
  schema_name: string;
  table_name: string;
  verdict: string;
  level: EngineLevel;
  checked: boolean;
  drill_down: RawDrillDownRow[] | null;
}

interface RawIgnored {
  id: string;
  name: string;
  reason: string;
}

interface RawCoverage {
  total_tables: number;
  full_values_tables: number;
  counts_tables: number;
  structure_tables: number;
  ignored_tables: number;
}

interface RawResults {
  run_id: string;
  verdict: string;
  coverage: RawCoverage;
  ignored: RawIgnored[];
  failures: RawFailureCard[];
  table_results: RawTableResult[];
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
    default:
      // `fuzzy`, `unmapped` and any unknown status are all "needs attention",
      // never an auto-approved exact/case match.
      return 'fuzzy';
  }
}

/** Only `exact` and `case_insensitive` are auto-approved. */
function isAutoApproved(status: string): boolean {
  return status === 'exact' || status === 'case_insensitive';
}

/** Project the engine `Mapping` + attention queue onto the contract `Mapping`. */
function translateMapping(
  build: RawMappingBuild,
  inventory: Inventory,
): Mapping {
  const matches: MappingMatch[] = [];
  const needsAttention: MappingMatch[] = [];

  const addMatch = (match: MappingMatch, status: string) => {
    matches.push(match);
    if (!isAutoApproved(status)) {
      // Fuzzy, unmapped and any unknown status require a human decision.
      needsAttention.push({ ...match, approved: false });
    }
  };

  for (const [schema, schemaMapping] of Object.entries(build.mapping.schema_mappings)) {
    for (const [oldTable, tableMapping] of Object.entries(schemaMapping.table_mappings)) {
      const tableStatus = tableMapping.status;
      if (tableStatus === 'ignored') {
        continue; // surfaced through build.mapping.ignored
      }
      addMatch(
        {
          id: `${schema}.${oldTable}`,
          oldName: `${schema}.${oldTable}`,
          newName: `${schemaMapping.target_schema}.${tableMapping.target_table}`,
          confidence: mapConfidence(tableStatus),
          approved: isAutoApproved(tableStatus),
        },
        tableStatus,
      );
      for (const [oldCol, colMapping] of Object.entries(tableMapping.column_mappings)) {
        const colStatus = colMapping.status;
        if (colStatus === 'ignored') {
          continue;
        }
        addMatch(
          {
            id: `${schema}.${oldTable}.${oldCol}`,
            oldName: `${oldTable}.${oldCol}`,
            newName: colMapping.target_column,
            confidence: mapConfidence(colStatus),
            approved: isAutoApproved(colStatus),
          },
          colStatus,
        );
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
  const encryption: EncryptionField = profile.encryption;
  return {
    id: profile.id,
    label: profile.label,
    host: profile.host,
    port: profile.port,
    database: profile.database,
    user: profile.user,
    encryption,
    tls_fingerprint: profile.tlsFingerprint ?? null,
    plaintext_reason: profile.plaintextReason ?? null,
    ca_file: profile.caFile ?? null,
  };
}

function toConnectInput(
  profile: ConnectionProfile,
  password?: string,
): {
  profile: BackendProfile;
  password: string | null;
} {
  // A typed password is a one-time value for this run, sent to the backend and
  // then dropped; it never enters app state (PLAN 6.5).  `null` tells the
  // backend to read the password from the keychain instead.
  return { profile: toBackendProfile(profile), password: password ?? null };
}

/** Map the backend's stored-profile shape back onto the frontend contract. */
function toConnectionProfile(raw: BackendProfile): ConnectionProfile {
  return {
    id: raw.id,
    label: raw.label,
    host: raw.host,
    port: raw.port,
    database: raw.database,
    user: raw.user,
    encryption: raw.encryption,
    tlsFingerprint: raw.tls_fingerprint ?? undefined,
    plaintextReason: raw.plaintext_reason ?? undefined,
  };
}

/** Map the backend's real TLS state, never hardcoding `verified: true`. */
function toTlsState(raw: RawTlsState): TlsState {
  if (raw.state === 'encrypted') {
    if (raw.verified) {
      return { state: 'encrypted', verified: true };
    }
    return { state: 'encrypted', verified: false, fingerprint: raw.fingerprint ?? '' };
  }
  if (raw.state === 'plaintext') {
    return { state: 'plaintext', reason: raw.reason };
  }
  return { state: 'unknown' };
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
    tls: toTlsState(raw.tls),
  };
}

/** Map the engine's snake_case check level onto the contract level. */
export function mapLevel(raw: EngineLevel): CheckLevel {
  switch (raw) {
    case 'structure':
      return 'structure';
    case 'counts_and_totals':
      return 'counts';
    case 'full_values':
      return 'full-values';
  }
}

/** Map the contract check level onto the engine's snake_case level. */
export function toEngineLevel(level: CheckLevel): EngineLevel {
  switch (level) {
    case 'structure':
      return 'structure';
    case 'counts':
      return 'counts_and_totals';
    case 'full-values':
      return 'full_values';
  }
}

/** Map a per-table progress state string onto the contract `TableRunState`. */
export function mapTableState(raw: string): TableRunState {
  switch (raw) {
    case 'running':
      return 'running';
    case 'paused':
      return 'paused';
    case 'done':
      return 'done';
    case 'failed':
      return 'failed';
    case 'cancelled':
      return 'cancelled';
    case 'changed-during-check':
      return 'changed-during-check';
    default:
      return 'queued';
  }
}

/** Map the run's overall state string onto the contract's union. */
export function mapOverallState(raw: string): RunProgress['overallState'] {
  switch (raw) {
    case 'paused':
      return 'paused';
    case 'done':
      return 'done';
    case 'cancelled':
      return 'cancelled';
    default:
      return 'running';
  }
}

/** Map a drill-down row kind (from the engine, always one of these three). */
function mapDrillKind(kind: string): DrillDownRow['kind'] {
  if (kind === 'extra') return 'extra';
  if (kind === 'changed') return 'changed';
  // "missing" and any unknown value both render as missing-in-new (never PASS);
  // the engine only emits these three, but stay conservative.
  return 'missing';
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
    if (e.kind === 'password_missing') {
      return 'no password is stored for this profile; type the password to connect this run';
    }
    if (typeof e.message === 'string') {
      return e.message;
    }
    return e.kind;
  }
  return error instanceof Error ? error.message : String(error);
}

// ---------------------------------------------------------------------------
// Data-half mapping (engine -> contract), all strict.
// ---------------------------------------------------------------------------

/** Fold the contract `CheckPlan` into the engine's `levels` map + control totals. */
function planToLevels(plan: CheckPlan): {
  levels: Record<string, EngineLevel>;
  controlTotalColumns: string[];
} {
  const levels: Record<string, EngineLevel> = {};
  const controls = new Set<string>();
  for (const table of plan.tables) {
    const path = `${table.schemaName}.${table.tableName}`;
    levels[path] = toEngineLevel(table.level);
    for (const column of table.controlTotalColumns ?? []) {
      controls.add(column);
    }
  }
  return { levels, controlTotalColumns: [...controls] };
}

function translateCheckPlan(raw: RawCheckPlan): CheckPlan {
  const tables: TableCheck[] = raw.tables.map((t) => ({
    tableId: t.table_id,
    schemaName: t.schema_name,
    tableName: t.table_name,
    level: mapLevel(t.level),
    controlTotalColumns: t.control_total_columns ?? undefined,
  }));
  return {
    tables,
    presets: raw.presets,
    estimateSecondsMin: raw.estimate_seconds_min,
    estimateSecondsMax: raw.estimate_seconds_max,
  };
}

function translateProgress(raw: RawRunProgress): RunProgress {
  if (raw.overall_state === 'failed') {
    const kind = raw.error?.kind ?? 'run_failed';
    const why = raw.error?.message ?? 'no detail was reported';
    throw new RunFailedError(kind, `The run failed and has no results: ${why}`);
  }
  const tables: TableProgress[] = raw.tables.map((t) => ({
    tableId: t.table_id,
    schemaName: t.schema_name,
    tableName: t.table_name,
    state: mapTableState(t.state),
    progressPercent: t.progress_percent,
    etaSeconds: t.eta_seconds ?? undefined,
  }));
  return {
    runId: raw.run_id,
    overallState: mapOverallState(raw.overall_state),
    tables,
    etaSecondsMin: raw.eta_seconds_min ?? undefined,
    etaSecondsMax: raw.eta_seconds_max ?? undefined,
  };
}

function translateResults(raw: RawResults): Results {
  const coverage: CoverageSummary = {
    totalTables: raw.coverage.total_tables,
    fullValuesTables: raw.coverage.full_values_tables,
    countsTables: raw.coverage.counts_tables,
    structureTables: raw.coverage.structure_tables,
    ignoredTables: raw.coverage.ignored_tables,
  };

  const ignored: IgnoreItem[] = raw.ignored.map((i) => ({
    id: i.id,
    name: i.name,
    reason: i.reason,
  }));

  const failures: FailureCard[] = raw.failures.map((f) => ({
    id: f.id,
    tableId: f.table_id,
    schemaName: f.schema_name,
    tableName: f.table_name,
    level: mapLevel(f.level),
    verdict: mapVerdict(f.verdict),
    summary: f.summary,
    detail: f.detail,
  }));

  const tableResults: TableResult[] = raw.table_results.map((t) => ({
    tableId: t.table_id,
    schemaName: t.schema_name,
    tableName: t.table_name,
    verdict: mapVerdict(t.verdict),
    level: mapLevel(t.level),
    checked: t.checked,
    drillDown: t.drill_down?.map((d) => ({
      key: d.key,
      kind: mapDrillKind(d.kind),
      differingColumns: d.differing_columns ?? undefined,
    })),
  }));

  return {
    runId: raw.run_id,
    verdict: mapVerdict(raw.verdict),
    coverage,
    ignored,
    failures,
    tableResults,
  };
}

// ---------------------------------------------------------------------------
// The engine.
// ---------------------------------------------------------------------------

/**
 * Create the Tauri-backed engine.
 *
 * Structure and data methods call the backend and map its serde output strictly;
 * only report export still throws a typed `EngineError` (NOT_AVAILABLE) until
 * T024 — never fabricating a verdict.
 *
 * The flow is linear (connect -> compare -> map), so the adapter remembers the
 * profiles used by `compareStructure` and reuses them in `buildMapping`, which
 * the backend computes against the two live databases.  The raw engine mapping
 * produced by `build_mapping` is kept for `saveMapping` and the data-half
 * commands.
 */
export function createTauriEngine(): Engine {
  let lastOldProfile: ConnectionProfile | null = null;
  let lastNewProfile: ConnectionProfile | null = null;
  let lastRawMapping: RawMapping | null = null;

  return {
    async testConnection(
      profile: ConnectionProfile,
      password?: string,
    ): Promise<ConnectionTestResult> {
      try {
        const info = await invoke<RawServerInfo>('probe_connection', {
          input: toConnectInput(profile, password),
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

    async saveProfile(
      profile: ConnectionProfile,
      password?: string,
    ): Promise<SaveProfileResult> {
      const result = await invoke<RawSaveProfileResult>('save_profile', {
        profile: toBackendProfile(profile),
        password: password ?? null,
      });
      return {
        profile: toConnectionProfile(result.profile),
        warning: result.warning ?? undefined,
      };
    },

    async profileHasPassword(id: string): Promise<boolean> {
      return invoke<boolean>('profile_has_password', { id });
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

    // --- Data half: wired to the real engine (never fabricate a verdict). ---

    async buildCheckPlan(): Promise<CheckPlan> {
      if (!lastOldProfile || !lastNewProfile) {
        throw new Error('compareStructure must run before buildCheckPlan');
      }
      if (!lastRawMapping) {
        throw new Error('buildMapping must run before buildCheckPlan');
      }
      const raw = await invoke<RawCheckPlan>('build_check_plan', {
        old: toConnectInput(lastOldProfile),
        new: toConnectInput(lastNewProfile),
        mapping: lastRawMapping,
      });
      return translateCheckPlan(raw);
    },

    async startRun(definition: RunDefinition): Promise<RunProgress> {
      if (!lastOldProfile || !lastNewProfile) {
        throw new Error('compareStructure must run before startRun');
      }
      if (!lastRawMapping) {
        throw new Error('buildMapping must run before startRun');
      }
      const { levels, controlTotalColumns } = planToLevels(definition.plan);
      const started = await invoke<RawStartRun>('start_run', {
        input: {
          old: toConnectInput(lastOldProfile),
          new: toConnectInput(lastNewProfile),
          mapping: lastRawMapping,
          levels,
          default_level: 'full_values',
          control_total_columns: controlTotalColumns,
        },
      });
      return { runId: started.run_id, overallState: 'running', tables: [] };
    },

    async pollRun(runId: string): Promise<RunProgress> {
      const raw = await invoke<RawRunProgress>('poll_run', { runId });
      return translateProgress(raw);
    },

    async setRunPaused(runId: string, paused: boolean): Promise<RunProgress> {
      const raw = await invoke<RawRunProgress>('set_run_paused', { runId, paused });
      return translateProgress(raw);
    },

    async cancelRun(runId: string): Promise<RunProgress> {
      const raw = await invoke<RawRunProgress>('cancel_run', { runId });
      return translateProgress(raw);
    },

    async getResults(runId: string): Promise<Results> {
      const raw = await invoke<RawResults>('get_results', { runId });
      return translateResults(raw);
    },

    exportReport(): Promise<ReportResult> {
      // Report export lands with T024; until then it is a typed NOT_AVAILABLE,
      // never a fabricated report or verdict.
      return notAvailable();
    },
    async getVersion(): Promise<string> {
      return invoke<string>('get_version');
    },
  };
}
