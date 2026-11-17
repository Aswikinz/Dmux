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
  /** Path to a private-CA PEM bundle (a bank's internal CA), used with 'verify'. */
  caFile?: string;
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
  /** True when the machine has no system keychain and Dmux keeps this
   *  password in memory until the app closes.  The UI never holds it. */
  sessionPassword?: boolean;
}

export interface ConnectionTestResult {
  ok: boolean;
  info?: ServerInfo;
  error?: string;
  /** The login is a superuser and Dmux refused it (no override yet). */
  superuserRefused?: boolean;
}

/** A user's explicit, recorded decision to allow a superuser login (PLAN 6.2).
 *  The reason is sent to the backend and written into the results. */
export interface SuperuserOverride {
  reason: string;
}

/** What `save_profile` returns: the stored profile plus, when the password could
 *  not be written to (or had to be removed from) the keychain, a plain-language
 *  warning.  The profile never carries a password value. */
export interface SaveProfileResult {
  profile: ConnectionProfile;
  /** Present when the password could not be stored, or when it was removed
   *  because the connection details changed without a new password. */
  warning?: string;
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
  /** Optional daily run window "HH:MM-HH:MM" (local time); the run waits
   *  between tables outside it. Absent when off. */
  runWindow?: string;
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
  /** Set while the run waits for its run window: the time it opens ("22:00"). */
  waitingForWindow?: string;
  /** Plain notices for the person watching the run. */
  notices?: string[];
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

/** A per-column note: a ticked option or a type-change flag (H2 / decision 23). */
export interface ColumnNote {
  column: string;
  /** Stable identifier, e.g. "option", "type_changed". */
  kind: string;
  /** Plain-language note shown in the report. */
  note: string;
}

export interface TableResult {
  tableId: string;
  schemaName: string;
  tableName: string;
  verdict: Verdict;
  level: CheckLevel;
  checked: boolean;
  drillDown?: DrillDownRow[];
  columnNotes?: ColumnNote[];
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
  /** Whether any column option was applied: shows "PASS (with options)". */
  optionsApplied: boolean;
  /** Whether the results file recorded per-column options (schema >= 3).  When
   *  false the UI shows "options not recorded", never a bare PASS. */
  optionsRecorded: boolean;
  /** Run-level warnings (can-write, superuser, row-level security, ...). */
  warnings: string[];
}

export type ReportFormat = 'pdf' | 'excel';

export interface ReportRequest {
  /** The finished run to report on.  The backend renders from its own copy of
   *  that run's results; the UI never sends results or a destination path. */
  runId: string;
  format: ReportFormat;
  /** Include row keys in the exported report (off by default). */
  includeKeys: boolean;
}

export interface ReportResult {
  ok: boolean;
  /** Where the report was saved (chosen in the native save dialog). */
  filePath?: string;
  /** The user dismissed the save dialog; nothing was written. */
  cancelled?: boolean;
  error?: string;
}

export interface RunDefinition {
  oldProfile: ConnectionProfile;
  newProfile: ConnectionProfile;
  mapping: Mapping;
  plan: CheckPlan;
}
