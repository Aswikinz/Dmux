import type {
  AppSettings,
  AppSettingsUpdate,
  CheckPlan,
  ConnectionProfile,
  ConnectionTestResult,
  HistoryEntry,
  OpenResultsResult,
  Inventory,
  Mapping,
  MappingEdit,
  MappingRule,
  ReportRequest,
  ReportResult,
  Results,
  RunDefinition,
  RunProgress,
  SaveProfileResult,
  RulePreview,
  ServerInfo,
  SuperuserOverride,
} from './contract';

/**
 * Engine interface that the Tauri backend will implement.
 *
 * The UI depends only on this interface, not on Tauri command details, so it
 * can be tested in a browser against the mock implementation.
 */
export interface Engine {
  /**
   * Test a connection profile and return server metadata.  `password`, when
   * present, is a one-time value the user typed for this run; it is never
   * stored and never enters app state (PLAN 6.5).  When omitted, the backend
   * reads the password from the OS keychain.
   */
  testConnection(profile: ConnectionProfile, password?: string): Promise<ConnectionTestResult>;

  /**
   * Save a profile.  The optional `password` is stored in the OS keychain only
   * (never SQLite); when it cannot be stored, the result carries a warning and
   * the app asks for the password on each run.
   */
  saveProfile(profile: ConnectionProfile, password?: string): Promise<SaveProfileResult>;

  /**
   * Allow (or, with `null`, stop allowing) a superuser login for every later
   * connection: test, structure, mapping, plan and run.  The reason is required
   * and is recorded in the results.  Superusers are refused by default.
   */
  setSuperuserOverride(override: SuperuserOverride | null): void;

  /** Every saved connection profile (never a password). */
  listProfiles(): Promise<ConnectionProfile[]>;

  /** Delete a saved profile together with its saved password. */
  deleteProfile(id: string): Promise<boolean>;

  /** Whether a keychain password is stored for a profile (never returns it). */
  profileHasPassword(id: string): Promise<boolean>;

  /** Compare the structure/inventory of the two connected databases. */
  compareStructure(
    oldProfile: ConnectionProfile,
    newProfile: ConnectionProfile,
  ): Promise<Inventory>;

  /** Propose an automatic mapping based on exact and case-insensitive names. */
  buildMapping(
    inventory: Inventory,
  ): Promise<Mapping>;

  /**
   * Change the mapping: accept a suggestion, pair an item by hand, or ignore
   * one.  The backend checks every name against the live databases and refuses
   * a second old table for the same new table; the refusal is the thrown
   * error's message.  Returns the updated mapping.
   */
  editMapping(mapping: Mapping, edits: MappingEdit[]): Promise<Mapping>;

  /** How many tables these rename rules would pair.  Changes nothing. */
  previewRules(rules: MappingRule[]): Promise<RulePreview>;

  /** Build the mapping again with these rules.  Choices made by hand are cleared. */
  rebuildMapping(rules: MappingRule[]): Promise<Mapping>;

  /** Save the mapping so it can be exported later. */
  saveMapping(mapping: Mapping): Promise<{ hash: string; json: string }>;

  /** Build a default check plan from the mapping. */
  buildCheckPlan(
    mapping: Mapping,
  ): Promise<CheckPlan>;

  /** Start a run and return the initial progress. */
  startRun(definition: RunDefinition): Promise<RunProgress>;

  /** Poll the current progress of a run. */
  pollRun(runId: string): Promise<RunProgress>;

  /** Pause or resume a run. */
  setRunPaused(runId: string, paused: boolean): Promise<RunProgress>;

  /** Cancel a run. */
  cancelRun(runId: string): Promise<RunProgress>;

  /** Fetch the final results after a run finishes. */
  getResults(runId: string): Promise<Results>;

  /** Export a report to the chosen path. */
  exportReport(request: ReportRequest): Promise<ReportResult>;

  /** The app settings (custom checks, scripts folder, Python, time limit). */
  getSettings(): Promise<AppSettings>;

  /** Change custom checks on/off or the time limit.  Never takes a path. */
  updateSettings(update: AppSettingsUpdate): Promise<AppSettings>;

  /** Open the native folder dialog (from the backend) to choose the scripts
   *  folder.  A dismissed dialog changes nothing. */
  chooseHooksFolder(): Promise<AppSettings>;

  clearHooksFolder(): Promise<AppSettings>;

  /** Open the native file dialog (from the backend) to choose the Python. */
  choosePython(): Promise<AppSettings>;

  /** Back to the bundled or automatically found Python. */
  clearPython(): Promise<AppSettings>;
  /** The local run history, newest first (metadata only). */
  listHistory(): Promise<HistoryEntry[]>;

  /** Load a history entry's results.  The backend verifies the stored file
   *  first and refuses one that was changed; only verified results come back. */
  openHistoryEntry(id: string): Promise<Results>;

  /** Choose a results file in a native dialog, verify it, and return it.  The
   *  returned `runId` works with `exportReport`. */
  openResultsFile(): Promise<OpenResultsResult>;

  /** A history entry's run log as readable text. */
  getHistoryLog(id: string): Promise<string>;

  /** Save a history entry's run log through a native save dialog. */
  saveHistoryLog(id: string): Promise<ReportResult>;

  /** Delete every stored results file and history row (profiles are kept).
   *  Resolves to how many runs were removed. */
  purgeHistory(): Promise<number>;

  /** Get the engine version string for the run log. */
  getVersion(): Promise<string>;
}

export type { ServerInfo };
