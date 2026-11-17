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

export type MatchConfidence = 'exact' | 'case' | 'fuzzy' | 'rule' | 'confirmed';

/** A table or column in the old database, as separate parts so a name that
 *  contains a dot is never split by guesswork. */
export interface MappingItemRef {
  schema: string;
  table: string;
  /** Present for a column. */
  column?: string;
}

export interface MappingMatch {
  id: string;
  oldName: string;
  newName: string;
  confidence: MatchConfidence;
  /** User has approved this match. */
  approved: boolean;
  /** Which old table or column this is, when the engine said (a real run). */
  ref?: MappingItemRef;
  /** New names the engine says could be the counterpart. */
  candidates?: string[];
}

/** A guess at the new counterpart of an unmatched item.  Never used until the
 *  user accepts it. */
export interface MappingSuggestion {
  /** The id of the queue item this applies to. */
  id: string;
  ref: MappingItemRef;
  /** A bare table or column name on the new side. */
  candidate: string;
  /** How alike the names are, 0 to 100. */
  confidence: number;
}

/** A table or column that exists only in the new database. */
export interface UnmatchedNewItem {
  id: string;
  /** For a column, `table` is the old table's new counterpart. */
  ref: MappingItemRef;
}

/** A rename rule.  Rules apply to table names only. */
export interface MappingRule {
  kind: 'schema_rename' | 'prefix_replace' | 'suffix_replace';
  pattern: string;
  target: string;
}

/** How many tables a set of rules would pair. */
export interface RulePreview {
  tablesTotal: number;
  tablesByRules: number;
  tablesMatched: number;
}

/** A change to the mapping, checked by the backend before it is applied. */
export type MappingEdit =
  | { kind: 'pair_table'; schema: string; table: string; newTable: string }
  | { kind: 'pair_column'; schema: string; table: string; column: string; newColumn: string }
  | { kind: 'ignore'; path: string; reason: string };

export interface IgnoreItem {
  id: string;
  name: string;
  reason: string;
}

export interface Mapping {
  /** Rules such as old schema -> new schema or prefix renames, in words. */
  rules: string[];
  /** The same rules in structured form (what the rule editor shows). */
  ruleSpecs?: MappingRule[];
  /** Guesses for unmatched items; used only when the user accepts one. */
  suggestions?: MappingSuggestion[];
  /** Items only the new database has: what a pairing picker can offer. */
  unmatchedNew?: UnmatchedNewItem[];
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
  /** Control-total columns the user has confirmed.  Only these are checked. */
  controlTotalColumns?: string[];
  /** Number columns Dmux suggests (names like amount or balance).  Nothing is
   *  checked until the user ticks one. */
  controlTotalSuggestions?: string[];
  /** Size on the old database, in bytes, when known. */
  sizeBytes?: number;
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
  /** Present only when custom checks ran for this run.  `verdict` above already
   *  includes any custom failure. */
  customChecks?: CustomChecks;
}

export type CustomSeverity = 'info' | 'warning' | 'fail';

export interface CustomFinding {
  id: string;
  severity: CustomSeverity;
  /** Already stripped of control characters by the backend; shown as text. */
  message: string;
  table?: string;
}

/** What the custom checks (the user's Python scripts) reported for a run. */
export interface CustomChecks {
  findings: CustomFinding[];
  /** Each script that ran, with the SHA-256 of the code that ran. */
  scripts: { name: string; sha256: string }[];
  /** Scripts that did not run, timed out or returned bad output. */
  warnings: string[];
}

/** One script in the chosen custom-checks folder. */
export interface ScriptEntry {
  name: string;
  /** False when Dmux refuses to run it; `reason` says why. */
  usable: boolean;
  reason?: string;
}

export interface FolderScan {
  /** Set when the whole folder is refused or unreadable. */
  folderProblem?: string;
  scripts: ScriptEntry[];
}

/** The app settings the Settings screen edits.  Appearance lives in the UI. */
export interface AppSettings {
  customChecksEnabled: boolean;
  /** Chosen with a native folder dialog opened by the backend. */
  hooksFolder?: string;
  folderScan?: FolderScan;
  /** A Python chosen with a native file dialog; absent means automatic. */
  pythonPath?: string;
  pythonSource: 'chosen' | 'bundled' | 'auto';
  /** The interpreter that would run now; absent when none can be found. */
  pythonFound?: string;
  timeoutSecs: number;
  /** The honest warning shown wherever custom checks are on. */
  warning: string;
  /** The check level a new plan starts from ('suggested' keeps the engine's own). */
  startingLevel: 'suggested' | 'structure' | 'counts' | 'full-values';
  /** Parallel scans per side (at least 1). */
  oldParallelism: number;
  newParallelism: number;
  /** Where the Report screen's "include row keys" box starts. */
  includeKeysDefault: boolean;
  appVersion: string;
  /** Where Dmux keeps its data (read-only). */
  dataDir: string;
  licence: string;
}

/** What the webview may change.  It never sends a path. */
export interface AppSettingsUpdate {
  customChecksEnabled?: boolean;
  timeoutSecs?: number;
  startingLevel?: AppSettings['startingLevel'];
  oldParallelism?: number;
  newParallelism?: number;
  includeKeysDefault?: boolean;
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
  /** The browser demo cannot save files: nothing was written. The desktop app
   *  opens the system's save dialog instead. */
  demoOnly?: boolean;
  error?: string;
}

export interface RunDefinition {
  oldProfile: ConnectionProfile;
  newProfile: ConnectionProfile;
  mapping: Mapping;
  plan: CheckPlan;
}

/** One finished run in the local history.  Metadata only: the verified results
 *  are loaded on demand with `openHistoryEntry`. */
export interface HistoryEntry {
  id: string;
  /** When the run started and finished, as ISO 8601 text (UTC). */
  startedAt: string;
  endedAt: string;
  verdict: Verdict;
  /** Whether column options were applied ("PASS (with options)"). */
  optionsApplied: boolean;
  /** Whether the run recorded its options. */
  optionsRecorded: boolean;
  /** The coverage line, e.g. "40 of 40 tables fully checked, ...". */
  coverage: string;
  oldLabel: string;
  newLabel: string;
  mappingHash: string;
  definitionHash: string;
  resultsSha256: string;
}

/** The outcome of "Open a results file": verified results, a dismissed dialog,
 *  or a plain refusal (for example a file that was changed after Dmux wrote it). */
export interface OpenResultsResult {
  ok: boolean;
  /** Present only when the file passed verification. */
  results?: Results;
  /** The user dismissed the open dialog; nothing was read. */
  cancelled?: boolean;
  error?: string;
}
