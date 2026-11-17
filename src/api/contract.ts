/**
 * Typed contract for the Dmux engine.
 *
 * This mirrors the planned engine interface. The Tauri backend will implement
 * `Engine` later; for now the UI is built and tested against deterministic
 * mock fixtures.
 */

export type EncryptionMode = 'verify' | 'trust' | 'allow-plaintext';

export interface ConnectionProfile {
  id: string;
  /** Human label shown in lists; not a connection string. */
  label: string;
  host: string;
  port: number;
  database: string;
  user: string;
  encryption: EncryptionMode;
  /** SHA-256 fingerprint of the certificate to trust once (encryption === 'trust'). */
  tlsFingerprint?: string;
  /** Recorded reason for an unencrypted connection (encryption === 'allow-plaintext'). */
  plaintextReason?: string;
  /** Stored in the OS keychain, never in UI state. */
  passwordFromKeychain?: boolean;
}

export type TlsState =
  | { state: 'encrypted'; verified: true }
  | { state: 'encrypted'; verified: false; fingerprint: string }
  | { state: 'plaintext'; reason?: string }
  | { state: 'unknown' };

export interface ServerInfo {
  version: string;
  encoding: string;
  collation: string;
  isSuperuser: boolean;
  canWrite: boolean;
  poolerSuspected: boolean;
  md5Available: boolean;
  tls: TlsState;
  /** Free-form warning text for the user, e.g. superuser refusal. */
  warning?: string;
}

export interface ConnectionTestResult {
  ok: boolean;
  info?: ServerInfo;
  error?: string;
}

export type DiffKind = 'missing' | 'added' | 'changed';

export interface SchemaDiffNode {
  id: string;
  kind: 'schema' | 'table' | 'column' | 'index' | 'constraint' | 'sequence' | 'view' | 'function' | 'extension' | 'trigger';
  name: string;
  status: DiffKind | 'same';
  /** Old value / type / definition for changed nodes. */
  old?: string;
  /** New value / type / definition for changed or added nodes. */
  new?: string;
  children?: SchemaDiffNode[];
}

export interface Inventory {
  schemas: SchemaDiffNode[];
  oldEncoding: string;
  newEncoding: string;
  oldCollation: string;
  newCollation: string;
}

export type MatchConfidence = 'exact' | 'case' | 'fuzzy';

export interface MappingMatch {
  id: string;
  oldName: string;
  newName: string;
  confidence: MatchConfidence;
  /** User has approved this match. */
  approved: boolean;
}

export interface IgnoreItem {
  id: string;
  name: string;
  reason: string;
}

export interface Mapping {
  /** Rules such as old schema -> new schema or prefix renames. */
  rules: string[];
  matches: MappingMatch[];
  /** Queue of items that need human attention. */
  needsAttention: MappingMatch[];
  ignored: IgnoreItem[];
}

export type CheckLevel = 'structure' | 'counts' | 'full-values';

export interface TableCheck {
  tableId: string;
  schemaName: string;
  tableName: string;
  level: CheckLevel;
  /** Column names proposed as control totals when level is counts or full-values. */
  controlTotalColumns?: string[];
}

export interface CheckPlan {
  tables: TableCheck[];
  presets: string[];
  /** Total estimated seconds, shown as a range. */
  estimateSecondsMin: number;
  estimateSecondsMax: number;
}

export type TableRunState =
  | 'queued'
  | 'running'
  | 'paused'
  | 'done'
  | 'failed'
  | 'cancelled'
  | 'changed-during-check';

export interface TableProgress {
  tableId: string;
  schemaName: string;
  tableName: string;
  state: TableRunState;
  progressPercent: number;
  /** Seconds remaining for this table, if known. */
  etaSeconds?: number;
}

export interface RunProgress {
  runId: string;
  overallState: 'running' | 'paused' | 'done' | 'cancelled';
  tables: TableProgress[];
  /** Overall seconds remaining, shown as a range. */
  etaSecondsMin?: number;
  etaSecondsMax?: number;
}

/**
 * The engine's results JSON emits exactly these verdict strings. A verdict is
 * a statement about the data (or the tool) once a table has been checked; the
 * run-progress "done"/"failed" states are not verdicts and never render as one.
 */
export type Verdict =
  | 'PASS'
  | 'FAIL'
  | 'WIDELY_DIFFERENT'
  | 'NEEDS_REVIEW'
  | 'CHANGED_DURING_CHECK'
  | 'TOOL_FAULT'
  | 'WARNING';

export interface FailureCard {
  id: string;
  tableId: string;
  schemaName: string;
  tableName: string;
  level: CheckLevel;
  /** The verdict this card explains; drives its icon + word badge. */
  verdict: Verdict;
  summary: string;
  detail: string;
}

export interface DrillDownRow {
  key: string;
  kind: 'missing' | 'extra' | 'changed';
  oldValues?: Record<string, string>;
  newValues?: Record<string, string>;
  differingColumns?: string[];
}

export interface TableResult {
  tableId: string;
  schemaName: string;
  tableName: string;
  verdict: Verdict;
  level: CheckLevel;
  checked: boolean;
  drillDown?: DrillDownRow[];
}

export interface CoverageSummary {
  totalTables: number;
  fullValuesTables: number;
  countsTables: number;
  structureTables: number;
  ignoredTables: number;
}

export interface Results {
  runId: string;
  verdict: Verdict;
  coverage: CoverageSummary;
  /** Tables/columns the user chose to ignore, each with its required reason. */
  ignored: IgnoreItem[];
  failures: FailureCard[];
  tableResults: TableResult[];
}

export type ReportFormat = 'pdf' | 'excel';

export interface ReportRequest {
  format: ReportFormat;
  /** Include row keys in the exported report (off by default). */
  includeKeys: boolean;
  /** Destination chosen by the user (Tauri dialog, T024/T040). Optional until
   *  the picker exists: no temp-path default is used, and the engine is free to
   *  choose a safe, user-triggered location. */
  filePath?: string;
}

export interface ReportResult {
  ok: boolean;
  filePath?: string;
  error?: string;
}

export interface RunDefinition {
  oldProfile: ConnectionProfile;
  newProfile: ConnectionProfile;
  mapping: Mapping;
  plan: CheckPlan;
}
