import type {
  CheckPlan,
  ConnectionProfile,
  ConnectionTestResult,
  Inventory,
  Mapping,
  ReportRequest,
  ReportResult,
  Results,
  RunDefinition,
  RunProgress,
  ServerInfo,
} from './contract';

/**
 * Engine interface that the Tauri backend will implement.
 *
 * The UI depends only on this interface, not on Tauri command details, so it
 * can be tested in a browser against the mock implementation.
 */
export interface Engine {
  /** Test a connection profile and return server metadata. */
  testConnection(profile: ConnectionProfile): Promise<ConnectionTestResult>;

  /** Compare the structure/inventory of the two connected databases. */
  compareStructure(
    oldProfile: ConnectionProfile,
    newProfile: ConnectionProfile,
  ): Promise<Inventory>;

  /** Propose an automatic mapping based on exact and case-insensitive names. */
  buildMapping(
    inventory: Inventory,
  ): Promise<Mapping>;

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

  /** Get the engine version string for the run log. */
  getVersion(): Promise<string>;
}

export type { ServerInfo };
