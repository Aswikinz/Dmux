/**
 * The real engine: the Tauri backend, reached through `@tauri-apps/api` invoke.
 *
 * Structure half (Connect / Compare structure / Map) calls the backend commands
 * and maps their serde (snake_case) JSON onto the frontend contract.  The data
 * half (checks / run / results) also calls the backend; each method maps the
 * engine's serde output strictly.  Report export asks the backend to render
 * its own copy of the run's results behind a native save dialog.  A false PASS is the worst bug this project can
 * have, so nothing here ever invents one.
 *
 * Every mapping is strict: an unknown status or verdict is treated as a
 * non-PASS result, never silently coerced to "same" or "PASS".
 */

import { invoke } from '@tauri-apps/api/core';
import type { Engine } from './engine';
import type {
  AppSettings,
  AppSettingsUpdate,
  CustomSeverity,
  CheckLevel,
  CheckPlan,
  ChangeDetectionState,
  ConnectionFact,
  ConnectionProfile,
  ConnectionTestResult,
  CoverageSummary,
  HistoryEntry,
  DrillDownRow,
  FailureCard,
  Inventory,
  Mapping,
  MappingMatch,
  MappingEdit,
  MappingRule,
  MappingSuggestion,
  RulePreview,
  UnmatchedNewItem,
  IgnoreItem,
  MatchConfidence,
  OpenResultsResult,
  ReportRequest,
  ReportResult,
  Results,
  RunDefinition,
  RunProgress,
  SaveProfileResult,
  SchemaDiffNode,
  ServerInfo,
  StructureRow,
  SuperuserOverride,
  TableCheck,
  TableFacts,
  TableProgress,
  TableResult,
  TableRunState,
  TlsState,
  Verdict,
  MappingSaveResult,
  CheckChoices,
  SaveChoicesResult,
  OpenChoicesResult,
  IssueKind,
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
  /** The password is kept in memory for this session (no keychain). */
  session_password?: boolean;
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
  /** Aligned side-by-side rows (absent from an older backend). */
  rows?: RawStructureRow[];
  old_server_version?: string;
  new_server_version?: string;
}

interface RawStructureRow {
  id: string;
  depth: number;
  kind: string;
  name: string;
  status: string;
  old: string | null;
  new: string | null;
  expandable: boolean;
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
  schema: string;
  table: string | null;
  column: string | null;
  issue: string;
  candidates: string[];
}

interface RawSuggestionEntry {
  path: string;
  schema: string;
  table: string | null;
  column: string | null;
  candidate: string;
  confidence: number;
}

/** `preview_rules` -> `RulePreview`. */
interface RawRulePreview {
  tables_total: number;
  tables_by_rules: number;
  tables_matched: number;
}

/** The `save_mapping` command's outcome. */
type RawSaveMapping = { status: 'saved'; path: string; hash: string } | { status: 'cancelled' };

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
/** Shape `export_report` returns (`commands::report::ExportReportResult`). */
type RawExportReport = { status: 'saved'; path: string } | { status: 'cancelled' };

/** `commands::history::OpenResultsOutcome`. */
type RawOpenResults = { status: 'opened'; results: RawResults } | { status: 'cancelled' };

/** `store::HistoryRow`. */
interface RawHistoryRow {
  id: string;
  started_at: number;
  ended_at: number;
  verdict: string;
  options_applied: boolean;
  options_recorded: boolean;
  coverage_line: string;
  old_label: string;
  new_label: string;
  mapping_hash: string;
  definition_hash: string;
  results_sha256: string;
}

function isoFromSeconds(seconds: number): string {
  return new Date(seconds * 1000).toISOString();
}

function translateHistoryRow(raw: RawHistoryRow): HistoryEntry {
  return {
    id: raw.id,
    startedAt: isoFromSeconds(raw.started_at),
    endedAt: isoFromSeconds(raw.ended_at),
    // Anything outside the engine vocabulary is a non-PASS tool fault.
    verdict: mapVerdict(raw.verdict),
    optionsApplied: raw.options_applied,
    optionsRecorded: raw.options_recorded,
    coverage: raw.coverage_line,
    oldLabel: raw.old_label,
    newLabel: raw.new_label,
    mappingHash: raw.mapping_hash,
    definitionHash: raw.definition_hash,
    resultsSha256: raw.results_sha256,
  };
}

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
  /** The columns Dmux suggests; none is a control total until confirmed. */
  control_total_columns: string[] | null;
  size_bytes?: number;
  columns?: { name: string; type_name: string; key: boolean; number: boolean }[];
}

/** `save_check_choices` / `open_check_choices`: the file's own snake_case form. */
interface RawTableChoice {
  table: string;
  level: EngineLevel;
  left_out_columns?: string[];
  control_totals?: string[];
}

interface RawCheckChoices {
  format: string;
  version: number;
  run_window?: string | null;
  tables: RawTableChoice[];
}

type RawSaveChoices = { status: 'saved'; path: string } | { status: 'cancelled' };
type RawOpenChoices =
  | { status: 'opened'; path: string; choices: RawCheckChoices }
  | { status: 'cancelled' };

/** Contract choices -> the file's form (the backend validates it again). */
export function toRawChoices(choices: CheckChoices): RawCheckChoices {
  return {
    format: 'dmux-check-choices',
    version: 1,
    run_window: choices.runWindow ?? null,
    tables: choices.tables.map((t) => ({
      table: t.table,
      level: toEngineLevel(t.level),
      left_out_columns: t.leftOutColumns,
      control_totals: t.controlTotals,
    })),
  };
}

/** The file's form (already validated by the backend) -> contract choices. */
export function fromRawChoices(raw: RawCheckChoices): CheckChoices {
  return {
    runWindow: raw.run_window ?? undefined,
    tables: raw.tables.map((t) => ({
      table: t.table,
      level: mapLevel(t.level),
      leftOutColumns: t.left_out_columns ?? [],
      controlTotals: t.control_totals ?? [],
    })),
  };
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
  waiting_for_window?: string | null;
  notices?: string[];
}

/** `get_results` -> `ResultsDto`. */
interface RawDrillDownRow {
  key: string;
  kind: string;
  differing_columns: string[] | null;
}

/** `get_results` -> `ColumnNoteDto`. */
interface RawColumnNote {
  column: string;
  kind: string;
  note: string;
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
  kind?: string;
}

interface RawTableResult {
  table_id: string;
  schema_name: string;
  table_name: string;
  verdict: string;
  level: EngineLevel;
  checked: boolean;
  kind?: string | null;
  drill_down: RawDrillDownRow[] | null;
  column_notes: RawColumnNote[];
  old_rows?: number | null;
  new_rows?: number | null;
  differing_buckets?: number | null;
  missing_buckets?: number | null;
  extra_buckets?: number | null;
  missing_rows?: number | null;
  extra_rows?: number | null;
  changed_rows?: number | null;
  reasons?: string[];
  facts?: RawTableFacts | null;
  ignored_columns?: RawIgnored[];
}

interface RawTableFacts {
  key_columns?: string[];
  columns?: { column: string; rule: string }[];
  buckets_total?: number | null;
  spot?: { state: string; compared?: number | null; differing?: number | null; note: string } | null;
  change_detection_old?: string | null;
  change_detection_new?: string | null;
  changed_during_scan?: boolean | null;
  control_totals?: { column: string; old: string; new: string }[];
}

interface RawConnectionInfo {
  tls: string;
  plaintext_reason?: string | null;
  superuser: boolean;
  superuser_reason?: string | null;
  can_write: boolean;
  server_version: string;
  server_encoding: string;
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
  options_applied: boolean;
  options_recorded: boolean;
  warnings: string[];
  tool_version?: string;
  mapping_hash?: string;
  results_sha256?: string;
  connections?: { old: RawConnectionInfo; new: RawConnectionInfo };
  custom_checks?: {
    findings: { id: string; severity: string; message: string; table: string | null }[];
    scripts: { name: string; sha256: string }[];
    warnings: string[];
  } | null;
}

interface RawSettings {
  custom_checks_enabled: boolean;
  hooks_folder: string | null;
  scan: {
    folder_problem: string | null;
    scripts: { name: string; usable: boolean; reason: string | null }[];
  } | null;
  python_path: string | null;
  python_source: 'chosen' | 'bundled' | 'auto';
  python_found: string | null;
  timeout_secs: number;
  warning: string;
  starting_level: string;
  old_parallelism: number;
  new_parallelism: number;
  include_keys_default: boolean;
  app_version: string;
  data_dir: string;
  licence: string;
}

/** Map the backend settings; text from the file system stays plain text. */
export function translateSettings(raw: RawSettings): AppSettings {
  return {
    customChecksEnabled: raw.custom_checks_enabled,
    hooksFolder: raw.hooks_folder ?? undefined,
    folderScan: raw.scan
      ? {
          folderProblem: raw.scan.folder_problem ?? undefined,
          scripts: raw.scan.scripts.map((s) => ({
            name: s.name,
            usable: s.usable,
            reason: s.reason ?? undefined,
          })),
        }
      : undefined,
    pythonPath: raw.python_path ?? undefined,
    pythonSource: raw.python_source,
    pythonFound: raw.python_found ?? undefined,
    timeoutSecs: raw.timeout_secs,
    warning: raw.warning,
    startingLevel: mapStartingLevel(raw.starting_level),
    oldParallelism: raw.old_parallelism,
    newParallelism: raw.new_parallelism,
    includeKeysDefault: raw.include_keys_default,
    appVersion: raw.app_version,
    dataDir: raw.data_dir,
    licence: raw.licence,
  };
}

/** An unknown starting level keeps the engine's own suggestion. */
function mapStartingLevel(raw: string): AppSettings['startingLevel'] {
  switch (raw) {
    case 'structure':
    case 'counts':
      return raw;
    case 'full_values':
      return 'full-values';
    default:
      return 'suggested';
  }
}

/** Run a settings command; a failure becomes a plain `Error` with readable text. */
async function settingsCall(command: string, args?: Record<string, unknown>): Promise<AppSettings> {
  try {
    return translateSettings(await invoke<RawSettings>(command, args));
  } catch (error) {
    throw new Error(describeError(error), { cause: error });
  }
}

/** Unknown custom-check severities are shown as failures, never hidden. */
function mapSeverity(raw: string): CustomSeverity {
  return raw === 'info' || raw === 'warning' ? raw : 'fail';
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

const STRUCTURE_STATUSES = new Set(['same', 'missing', 'added', 'changed']);

/** One aligned row.  An unknown status is shown as changed, never as same. */
export function translateStructureRow(r: RawStructureRow): StructureRow {
  return {
    id: r.id,
    depth: r.depth,
    kind: r.kind as StructureRow['kind'],
    name: r.name,
    status: (STRUCTURE_STATUSES.has(r.status) ? r.status : 'changed') as StructureRow['status'],
    old: r.old ?? undefined,
    new: r.new ?? undefined,
    expandable: r.expandable,
  };
}

/** Convert an engine mapping status to the contract `MatchConfidence`. */
function mapConfidence(status: string): MatchConfidence {
  switch (status) {
    case 'exact':
      return 'exact';
    case 'case_insensitive':
      return 'case';
    case 'rule':
      return 'rule';
    case 'confirmed':
      return 'confirmed';
    default:
      // `fuzzy`, `unmapped` and any unknown status are all "needs attention",
      // never an auto-approved exact/case match.
      return 'fuzzy';
  }
}

/** Exact, case-insensitive, rule and user-confirmed pairings need no decision. */
function isAutoApproved(status: string): boolean {
  return (
    status === 'exact' || status === 'case_insensitive' || status === 'rule' || status === 'confirmed'
  );
}

/** The `edit_mapping` wire form of an edit (snake_case fields). */
function toRawEdit(edit: MappingEdit): Record<string, string> {
  switch (edit.kind) {
    case 'pair_table':
      return {
        kind: 'pair_table',
        schema: edit.schema,
        table: edit.table,
        new_table: edit.newTable,
      };
    case 'pair_column':
      return {
        kind: 'pair_column',
        schema: edit.schema,
        table: edit.table,
        column: edit.column,
        new_column: edit.newColumn,
      };
    case 'ignore':
      return { kind: 'ignore', path: edit.path, reason: edit.reason };
  }
}

/**
 * Lay an edited engine mapping over the queue the build produced: an item that
 * is now paired or ignored leaves the queue, a new item that is now taken
 * leaves the picker's list, and a suggestion goes with its item.
 */
function pruneBuild(build: RawMappingBuild, raw: RawMapping): RawMappingBuild {
  const ignored = new Set(raw.ignored.map((i) => i.path));
  const tableOf = (schema: string, table: string) =>
    raw.schema_mappings[schema]?.table_mappings[table];
  const paired = (e: { schema: string; table: string | null; column: string | null }) => {
    if (!e.table) return false;
    const tm = tableOf(e.schema, e.table);
    if (!tm) return false;
    return e.column ? e.column in tm.column_mappings : true;
  };
  const taken = (e: { schema: string; table: string | null; column: string | null }) => {
    if (!e.table) return false;
    if (e.column) {
      const tm = tableOf(e.schema, e.table);
      return Object.values(tm?.column_mappings ?? {}).some((c) => c.target_column === e.column);
    }
    return Object.values(raw.schema_mappings[e.schema]?.table_mappings ?? {}).some(
      (t) => t.target_table === e.table,
    );
  };
  const needs_attention = build.needs_attention.filter((a) =>
    a.issue === 'add_back_review'
      ? !taken(a) && !ignored.has(a.path)
      : !paired(a) && !ignored.has(a.path),
  );
  const kept = new Set(
    needs_attention.filter((a) => a.issue !== 'add_back_review').map((a) => a.path),
  );
  const suggestions = build.suggestions.filter(
    (s) =>
      kept.has(s.path) &&
      !taken({ schema: s.schema, table: s.column ? s.table : s.candidate, column: s.column ? s.candidate : null }),
  );
  return { mapping: raw, needs_attention, suggestions };
}

function isRuleKind(kind: string): kind is MappingRule['kind'] {
  return kind === 'schema_rename' || kind === 'prefix_replace' || kind === 'suffix_replace';
}

/** Project the engine `Mapping` + attention queue onto the contract `Mapping`. */
function translateMapping(
  build: RawMappingBuild,
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

  const ignoredPaths = new Set(build.mapping.ignored.map((i) => i.path));
  for (const [schema, schemaMapping] of Object.entries(build.mapping.schema_mappings)) {
    for (const [oldTable, tableMapping] of Object.entries(schemaMapping.table_mappings)) {
      const tableStatus = tableMapping.status;
      if (tableStatus === 'ignored' || ignoredPaths.has(`${schema}.${oldTable}`)) {
        continue; // surfaced through build.mapping.ignored
      }
      addMatch(
        {
          id: `${schema}.${oldTable}`,
          oldName: `${schema}.${oldTable}`,
          newName: `${schemaMapping.target_schema}.${tableMapping.target_table}`,
          confidence: mapConfidence(tableStatus),
          approved: isAutoApproved(tableStatus),
          kind: 'table',
          group: `${schema}.${oldTable}`,
        },
        tableStatus,
      );
      for (const [oldCol, colMapping] of Object.entries(tableMapping.column_mappings)) {
        const colStatus = colMapping.status;
        if (colStatus === 'ignored' || ignoredPaths.has(`${schema}.${oldTable}.${oldCol}`)) {
          continue;
        }
        addMatch(
          {
            id: `${schema}.${oldTable}.${oldCol}`,
            oldName: `${oldTable}.${oldCol}`,
            newName: colMapping.target_column,
            confidence: mapConfidence(colStatus),
            approved: isAutoApproved(colStatus),
            kind: 'column',
            group: `${schema}.${oldTable}`,
            oldLabel: oldCol,
            newLabel: colMapping.target_column,
          },
          colStatus,
        );
      }
    }
  }

  const unmatchedNew: UnmatchedNewItem[] = [];
  for (const attention of build.needs_attention) {
    const ref = attention.table
      ? {
          schema: attention.schema,
          table: attention.table,
          ...(attention.column ? { column: attention.column } : {}),
        }
      : undefined;
    if (attention.issue === 'add_back_review') {
      // Only the new database has it: not a decision for the user, but a
      // counterpart the pairing picker can offer for an unmatched old item.
      if (ref) unmatchedNew.push({ id: attention.path, ref });
      continue;
    }
    // Nothing is paired yet, so there is no new name to approve.  The user
    // accepts a suggestion or picks a counterpart.
    needsAttention.push({
      id: attention.path,
      oldName: attention.path,
      newName: '',
      confidence: 'fuzzy',
      approved: false,
      ...(ref ? { ref } : {}),
      candidates: attention.candidates,
    });
  }

  const suggestions: MappingSuggestion[] = [];
  for (const s of build.suggestions) {
    if (!s.table) continue;
    suggestions.push({
      id: s.path,
      ref: {
        schema: s.schema,
        table: s.table,
        ...(s.column ? { column: s.column } : {}),
      },
      candidate: s.candidate,
      confidence: s.confidence,
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

  // An encoding difference is not a mapping decision: Dmux converts every
  // value to UTF-8 before comparing.  It is shown as information on Compare
  // structure, never as a blocking "needs attention" item.
  const ruleSpecs: MappingRule[] = build.mapping.rules.flatMap((rule) =>
    isRuleKind(rule.kind)
      ? [{ kind: rule.kind, pattern: rule.pattern, target: rule.target }]
      : [],
  );
  return { rules, ruleSpecs, matches, needsAttention, ignored, suggestions, unmatchedNew };
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
    caFile: raw.ca_file ?? undefined,
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
    sessionPassword: raw.session_password === true,
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
    case 'not-scanned':
      return 'not-scanned';
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
/** Plain text for anything a backend call or a screen can throw: a typed
 *  backend error, an Error, or anything else.  Never "[object Object]". */
export function describeError(error: unknown): string {
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
    if (e.kind === 'password_remove_failed') {
      return 'The connection works, but it could not be saved because the system keychain could not be updated. Dmux keeps the password in memory until you close the app.';
    }
    if (typeof e.message === 'string') {
      return e.message;
    }
    return e.kind;
  }
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Something went wrong, and Dmux could not describe it.';
}

// ---------------------------------------------------------------------------
// Data-half mapping (engine -> contract), all strict.
// ---------------------------------------------------------------------------

/** Fold the contract `CheckPlan` into the engine's `levels` map, control
 *  totals and left-out columns.  Control totals are named `schema.table.column`
 *  so each one applies to its own table only.  A key column is never sent as
 *  left out (the engine would compare it anyway). */
export function planToLevels(plan: CheckPlan): {
  levels: Record<string, EngineLevel>;
  controlTotalColumns: string[];
  skipColumns: string[];
} {
  const levels: Record<string, EngineLevel> = {};
  const controls: string[] = [];
  const skip: string[] = [];
  for (const table of plan.tables) {
    if (table.included === false) continue;
    const path = `${table.schemaName}.${table.tableName}`;
    levels[path] = toEngineLevel(table.level);
    for (const column of table.controlTotalColumns ?? []) {
      controls.push(`${path}.${column}`);
    }
    const keys = new Set((table.columns ?? []).filter((c) => c.key).map((c) => c.name));
    for (const column of table.leftOutColumns ?? []) {
      if (!keys.has(column)) skip.push(`${path}.${column}`);
    }
  }
  return { levels, controlTotalColumns: controls, skipColumns: skip };
}

function translateCheckPlan(raw: RawCheckPlan): CheckPlan {
  const tables: TableCheck[] = raw.tables.map((t) => ({
    tableId: t.table_id,
    schemaName: t.schema_name,
    tableName: t.table_name,
    level: mapLevel(t.level),
    // The backend's list is what it *suggests*; nothing is confirmed yet.
    controlTotalSuggestions: t.control_total_columns ?? undefined,
    sizeBytes: t.size_bytes ?? undefined,
    columns: t.columns?.map((c) => ({
      name: c.name,
      typeName: c.type_name,
      key: c.key,
      number: c.number,
    })),
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
    waitingForWindow: raw.waiting_for_window ?? undefined,
    notices: raw.notices && raw.notices.length > 0 ? raw.notices : undefined,
  };
}

const CHANGE_STATES = new Set(['available', 'track_counts_off', 'foreign_table']);

/** An unknown change-detection word is treated as unavailable, never as fine. */
function mapChangeState(s: string | null | undefined): ChangeDetectionState | undefined {
  if (s == null) return undefined;
  return CHANGE_STATES.has(s) ? (s as ChangeDetectionState) : 'track_counts_off';
}

function translateFacts(f: RawTableFacts): TableFacts {
  const spotState = f.spot?.state;
  return {
    keyColumns: f.key_columns ?? [],
    columns: f.columns ?? [],
    bucketsTotal: f.buckets_total ?? undefined,
    // An unrecognised spot state is shown as "disagreed": never a quiet agree.
    spot: f.spot
      ? {
          state: spotState === 'agreed' || spotState === 'not_run' ? spotState : 'disagreed',
          compared: f.spot.compared ?? undefined,
          differing: f.spot.differing ?? undefined,
          note: f.spot.note,
        }
      : undefined,
    changeDetectionOld: mapChangeState(f.change_detection_old),
    changeDetectionNew: mapChangeState(f.change_detection_new),
    changedDuringScan: f.changed_during_scan ?? undefined,
    controlTotals: f.control_totals ?? [],
  };
}

function translateConnection(c: RawConnectionInfo): ConnectionFact {
  return {
    tls: c.tls,
    plaintextReason: c.plaintext_reason ?? undefined,
    superuser: c.superuser,
    superuserReason: c.superuser_reason ?? undefined,
    canWrite: c.can_write,
    serverVersion: c.server_version,
    serverEncoding: c.server_encoding,
  };
}

/** The engine's issue kind, or undefined when it did not say. */
export function mapIssueKind(kind: string | null | undefined): IssueKind | undefined {
  return kind === 'data' || kind === 'structure' || kind === 'other' || kind === 'not_mapped'
    ? kind
    : undefined;
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
    // An unknown kind is never guessed as "data": it is shown as other.
    kind: mapIssueKind(f.kind) ?? 'other',
  }));

  const tableResults: TableResult[] = raw.table_results.map((t) => ({
    tableId: t.table_id,
    schemaName: t.schema_name,
    tableName: t.table_name,
    verdict: mapVerdict(t.verdict),
    level: mapLevel(t.level),
    checked: t.checked,
    ...(mapIssueKind(t.kind) ? { kind: mapIssueKind(t.kind) } : {}),
    drillDown: t.drill_down?.map((d) => ({
      key: d.key,
      kind: mapDrillKind(d.kind),
      differingColumns: d.differing_columns ?? undefined,
    })),
    columnNotes: (t.column_notes ?? []).map((n) => ({
      column: n.column,
      kind: n.kind,
      note: n.note,
    })),
    oldRows: t.old_rows ?? undefined,
    newRows: t.new_rows ?? undefined,
    differingBuckets: t.differing_buckets ?? undefined,
    missingBuckets: t.missing_buckets ?? undefined,
    extraBuckets: t.extra_buckets ?? undefined,
    missingRows: t.missing_rows ?? undefined,
    extraRows: t.extra_rows ?? undefined,
    changedRows: t.changed_rows ?? undefined,
    reasons: t.reasons ?? [],
    facts: t.facts ? translateFacts(t.facts) : undefined,
    ignoredColumns: (t.ignored_columns ?? []).map((i) => ({ id: i.id, name: i.name, reason: i.reason })),
  }));

  return {
    runId: raw.run_id,
    verdict: mapVerdict(raw.verdict),
    coverage,
    ignored,
    failures,
    tableResults,
    optionsApplied: raw.options_applied ?? false,
    // A v2 file never recorded options: default to false so the UI says
    // "options not recorded" rather than assume none (never a bare PASS).
    optionsRecorded: raw.options_recorded ?? false,
    warnings: raw.warnings ?? [],
    runFacts: raw.connections
      ? {
          toolVersion: raw.tool_version ?? '',
          mappingHash: raw.mapping_hash ?? '',
          resultsSha256: raw.results_sha256 ?? '',
          oldConnection: translateConnection(raw.connections.old),
          newConnection: translateConnection(raw.connections.new),
        }
      : undefined,
    customChecks: raw.custom_checks
      ? {
          findings: raw.custom_checks.findings.map((f) => ({
            id: f.id,
            severity: mapSeverity(f.severity),
            message: f.message,
            table: f.table ?? undefined,
          })),
          scripts: raw.custom_checks.scripts,
          warnings: raw.custom_checks.warnings,
        }
      : undefined,
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
  /** The build behind `lastRawMapping`: its queue, suggestions and new-only items. */
  let lastBuild: RawMappingBuild | null = null;
  let superuserOverride: SuperuserOverride | null = null;
  /** The backend's `SuperuserOverride { allow, reason }`, or null (refuse). */
  const allowSuperuser = () =>
    superuserOverride && superuserOverride.reason.trim()
      ? { allow: true, reason: superuserOverride.reason.trim() }
      : null;

  return {
    setSuperuserOverride(override: SuperuserOverride | null): void {
      superuserOverride = override && override.reason.trim() ? { reason: override.reason } : null;
    },

    async testConnection(
      profile: ConnectionProfile,
      password?: string,
    ): Promise<ConnectionTestResult> {
      try {
        const info = await invoke<RawServerInfo>('probe_connection', {
          input: toConnectInput(profile, password),
          allowSuperuser: allowSuperuser(),
        });
        const serverInfo = toServerInfo(info);
        if (serverInfo.isSuperuser) {
          serverInfo.warning =
            'This login is a superuser. Dmux refuses superuser logins by default.';
        }
        return { ok: true, info: serverInfo };
      } catch (error) {
        const refused =
          typeof error === 'object' &&
          error !== null &&
          (error as { kind?: unknown }).kind === 'superuser_refused';
        return {
          ok: false,
          error: describeError(error),
          ...(refused ? { superuserRefused: true } : {}),
        };
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

    async listProfiles(): Promise<ConnectionProfile[]> {
      const rows = await invoke<BackendProfile[]>('list_profiles');
      return rows.map(toConnectionProfile);
    },

    async deleteProfile(id: string): Promise<boolean> {
      return invoke<boolean>('delete_profile', { id });
    },

    async compareStructure(
      oldProfile: ConnectionProfile,
      newProfile: ConnectionProfile,
    ): Promise<Inventory> {
      lastOldProfile = oldProfile;
      lastNewProfile = newProfile;

      // One call: the engine collects and compares both inventories itself.
      // The raw inventories hold 64-bit values (sequence bounds up to
      // i64::MAX) that a JS number cannot carry, so they must not round-trip
      // through the webview.
      const diff = await invoke<RawSchemaDiff>('compare_structure', {
        old: toConnectInput(oldProfile),
        new: toConnectInput(newProfile),
        allowSuperuser: allowSuperuser(),
      });

      return {
        schemas: translateDiff(diff.entries),
        oldEncoding: diff.old_encoding,
        newEncoding: diff.new_encoding,
        oldCollation: diff.old_collation,
        newCollation: diff.new_collation,
        rows: diff.rows?.map(translateStructureRow),
        oldServerVersion: diff.old_server_version,
        newServerVersion: diff.new_server_version,
      };
    },

    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the backend builds the mapping from the live databases
    async buildMapping(_inventory: Inventory): Promise<Mapping> {
      if (!lastOldProfile || !lastNewProfile) {
        throw new Error('compareStructure must run before buildMapping');
      }
      const build = await invoke<RawMappingBuild>('build_mapping', {
        old: toConnectInput(lastOldProfile),
        new: toConnectInput(lastNewProfile),
        allowSuperuser: allowSuperuser(),
      });
      lastRawMapping = build.mapping;
      lastBuild = build;
      return translateMapping(build);
    },

    // The webview's mapping is a projection and is never sent back: the backend
    // changes its own copy, after checking every name against the live databases.
    async editMapping(_mapping: Mapping, edits: MappingEdit[]): Promise<Mapping> {
      if (!lastOldProfile || !lastNewProfile || !lastRawMapping) {
        throw new Error('buildMapping must run before editMapping');
      }
      // A refusal (such as two old tables for one new table) is thrown as its
      // plain-language message and leaves the mapping as it was.
      let raw: RawMapping;
      try {
        raw = await invoke<RawMapping>('edit_mapping', {
          old: toConnectInput(lastOldProfile),
          new: toConnectInput(lastNewProfile),
          mapping: lastRawMapping,
          edits: edits.map(toRawEdit),
          allowSuperuser: allowSuperuser(),
        });
      } catch (error) {
        throw new Error(describeError(error), { cause: error });
      }
      lastRawMapping = raw;
      lastBuild = pruneBuild(lastBuild ?? { mapping: raw, needs_attention: [], suggestions: [] }, raw);
      return translateMapping(lastBuild);
    },

    async previewRules(rules: MappingRule[]): Promise<RulePreview> {
      if (!lastOldProfile || !lastNewProfile) {
        throw new Error('compareStructure must run before previewRules');
      }
      try {
        const raw = await invoke<RawRulePreview>('preview_rules', {
          old: toConnectInput(lastOldProfile),
          new: toConnectInput(lastNewProfile),
          rules,
          allowSuperuser: allowSuperuser(),
        });
        return {
          tablesTotal: raw.tables_total,
          tablesByRules: raw.tables_by_rules,
          tablesMatched: raw.tables_matched,
        };
      } catch (error) {
        throw new Error(describeError(error), { cause: error });
      }
    },

    async rebuildMapping(rules: MappingRule[]): Promise<Mapping> {
      if (!lastOldProfile || !lastNewProfile) {
        throw new Error('compareStructure must run before rebuildMapping');
      }
      try {
        const build = await invoke<RawMappingBuild>('rebuild_mapping', {
          old: toConnectInput(lastOldProfile),
          new: toConnectInput(lastNewProfile),
          rules,
          allowSuperuser: allowSuperuser(),
        });
        lastRawMapping = build.mapping;
        lastBuild = build;
        return translateMapping(build);
      } catch (error) {
        throw new Error(describeError(error), { cause: error });
      }
    },

    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the projected mapping is rebuilt from the backend `lastRawMapping`
    async saveMapping(_mapping: Mapping): Promise<MappingSaveResult> {
      if (!lastRawMapping) {
        throw new Error('buildMapping must run before saveMapping');
      }
      // The backend validates the mapping, opens the native save dialog and
      // writes the file itself: no path crosses the IPC.
      try {
        const raw = await invoke<RawSaveMapping>('save_mapping', { mapping: lastRawMapping });
        if (raw.status === 'cancelled') return { ok: false, cancelled: true };
        return { ok: true, filePath: raw.path, hash: raw.hash };
      } catch (error) {
        return { ok: false, error: describeError(error) };
      }
    },

    async saveCheckChoices(choices: CheckChoices): Promise<SaveChoicesResult> {
      // The backend validates the choices, opens the save dialog and writes
      // the file itself: no path crosses the IPC.
      const raw = await invoke<RawSaveChoices>('save_check_choices', {
        choices: toRawChoices(choices),
      });
      return raw.status === 'saved' ? { status: 'saved', path: raw.path } : { status: 'cancelled' };
    },

    async openCheckChoices(): Promise<OpenChoicesResult> {
      const raw = await invoke<RawOpenChoices>('open_check_choices');
      return raw.status === 'opened'
        ? { status: 'opened', path: raw.path, choices: fromRawChoices(raw.choices) }
        : { status: 'cancelled' };
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
        allowSuperuser: allowSuperuser(),
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
      const { levels, controlTotalColumns, skipColumns } = planToLevels(definition.plan);
      const started = await invoke<RawStartRun>('start_run', {
        input: {
          old: toConnectInput(lastOldProfile),
          new: toConnectInput(lastNewProfile),
          mapping: lastRawMapping,
          levels,
          default_level: 'full_values',
          control_total_columns: controlTotalColumns,
          allow_superuser: allowSuperuser(),
          run_window: definition.plan.runWindow ?? null,
          // Tables the user left out: the backend records each as ignored
          // ("not selected for this run"), so none is dropped silently.
          skip_tables: definition.plan.tables
            .filter((t) => t.included === false)
            .map((t) => `${t.schemaName}.${t.tableName}`),
          // Columns left out of chosen tables, recorded the same way.
          skip_columns: skipColumns,
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

    async exportReport(request: ReportRequest): Promise<ReportResult> {
      // The backend renders from its own copy of the run's results and opens
      // the native save dialog itself: no results and no path cross the IPC.
      try {
        const raw = await invoke<RawExportReport>('export_report', {
          input: {
            run_id: request.runId,
            format: request.format,
            include_keys: request.includeKeys,
            leave_out: request.leaveOut ?? {
              structure: false,
              other: false,
              warnings: false,
              passing: false,
            },
          },
        });
        if (raw.status === 'cancelled') {
          return { ok: false, cancelled: true };
        }
        return { ok: true, filePath: raw.path };
      } catch (error) {
        return { ok: false, error: describeError(error) };
      }
    },
    async getSettings(): Promise<AppSettings> {
      return settingsCall('get_settings');
    },
    async updateSettings(update: AppSettingsUpdate): Promise<AppSettings> {
      return settingsCall('update_settings', {
        input: {
          custom_checks_enabled: update.customChecksEnabled,
          timeout_secs: update.timeoutSecs,
          starting_level:
            update.startingLevel === 'full-values' ? 'full_values' : update.startingLevel,
          old_parallelism: update.oldParallelism,
          new_parallelism: update.newParallelism,
          include_keys_default: update.includeKeysDefault,
        },
      });
    },
    async chooseHooksFolder(): Promise<AppSettings> {
      return settingsCall('choose_hooks_folder');
    },
    async clearHooksFolder(): Promise<AppSettings> {
      return settingsCall('clear_hooks_folder');
    },
    async choosePython(): Promise<AppSettings> {
      return settingsCall('choose_python');
    },
    async clearPython(): Promise<AppSettings> {
      return settingsCall('clear_python');
    },
    async listHistory(): Promise<HistoryEntry[]> {
      const rows = await invoke<RawHistoryRow[]>('list_history');
      return rows.map(translateHistoryRow);
    },

    async openHistoryEntry(id: string): Promise<Results> {
      // The backend verifies the stored file before it returns anything.
      const raw = await invoke<RawResults>('open_history_entry', { id });
      return translateResults(raw);
    },

    async openResultsFile(): Promise<OpenResultsResult> {
      // The native open dialog, the read and the verification all happen in
      // the backend; no path and no file content cross the IPC.
      try {
        const raw = await invoke<RawOpenResults>('open_results_file');
        if (raw.status === 'cancelled') {
          return { ok: false, cancelled: true };
        }
        return { ok: true, results: translateResults(raw.results) };
      } catch (error) {
        return { ok: false, error: describeError(error) };
      }
    },

    async getHistoryLog(id: string): Promise<string> {
      return invoke<string>('get_history_log', { id });
    },

    async saveHistoryLog(id: string): Promise<ReportResult> {
      try {
        const raw = await invoke<RawExportReport>('save_history_log', { id });
        if (raw.status === 'cancelled') {
          return { ok: false, cancelled: true };
        }
        return { ok: true, filePath: raw.path };
      } catch (error) {
        return { ok: false, error: describeError(error) };
      }
    },

    async purgeHistory(): Promise<number> {
      return invoke<number>('purge_history');
    },

    async getVersion(): Promise<string> {
      return invoke<string>('get_version');
    },
  };
}
