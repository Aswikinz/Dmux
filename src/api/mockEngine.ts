import type { Engine } from './engine';
import type {
  CheckPlan,
  ConnectionProfile,
  ConnectionTestResult,
  ConnectionFact,
  CoverageSummary,
  DrillDownRow,
  FailureCard,
  IgnoreItem,
  Inventory,
  Mapping,
  MappingMatch,
  ReportRequest,
  ReportResult,
  Results,
  RunDefinition,
  RunProgress,
  SaveProfileResult,
  ServerInfo,
  StructureRow,
  TableCheck,
  TableFacts,
  TableProgress,
  TableResult,
  TableRunState,
  Verdict,
} from './contract';

const deterministicId = (prefix: string, index: number) =>
  `${prefix}-${index.toString().padStart(6, '0')}`;

const version = '0.0.1-mock';

const makeServerInfo = (overrides?: Partial<ServerInfo>): ServerInfo => ({
  version: '17.0 (mock)',
  encoding: 'UTF8',
  collation: 'en_US.utf8',
  isSuperuser: false,
  canWrite: false,
  poolerSuspected: false,
  md5Available: true,
  tls: { state: 'encrypted', verified: true },
  ...overrides,
});

const baseInventory = (): Inventory => ({
  schemas: [
    {
      id: 'sch-demo_bank',
      kind: 'schema',
      name: 'demo_bank',
      status: 'same',
      children: [
        {
          id: 'tbl-accounts',
          kind: 'table',
          name: 'accounts',
          status: 'same',
          children: [
            { id: 'col-id', kind: 'column', name: 'id', status: 'same', old: 'bigint', new: 'bigint' },
            { id: 'col-name', kind: 'column', name: 'name', status: 'same', old: 'varchar(100)', new: 'varchar(100)' },
            { id: 'col-balance', kind: 'column', name: 'balance', status: 'same', old: 'numeric(18,2)', new: 'numeric(18,2)' },
          ],
        },
        {
          id: 'tbl-transactions',
          kind: 'table',
          name: 'transactions',
          status: 'same',
          children: [
            { id: 'col-tx-id', kind: 'column', name: 'id', status: 'same', old: 'bigint', new: 'bigint' },
            { id: 'col-account-id', kind: 'column', name: 'account_id', status: 'same', old: 'bigint', new: 'bigint' },
            { id: 'col-amount', kind: 'column', name: 'amount', status: 'same', old: 'numeric(18,2)', new: 'numeric(18,2)' },
            { id: 'col-posted-at', kind: 'column', name: 'posted_at', status: 'same', old: 'timestamp', new: 'timestamp' },
          ],
        },
      ],
    },
  ],
  oldEncoding: 'UTF8',
  newEncoding: 'UTF8',
  oldCollation: 'en_US.utf8',
  newCollation: 'en_US.utf8',
});

const diffInventory = (): Inventory => ({
  schemas: [
    {
      id: 'sch-demo_bank',
      kind: 'schema',
      name: 'demo_bank',
      status: 'same',
      children: [
        {
          id: 'tbl-accounts',
          kind: 'table',
          name: 'accounts',
          status: 'same',
          children: [
            { id: 'col-id', kind: 'column', name: 'id', status: 'same', old: 'bigint', new: 'bigint' },
            { id: 'col-name', kind: 'column', name: 'name', status: 'same', old: 'varchar(100)', new: 'varchar(100)' },
            {
              id: 'col-balance',
              kind: 'column',
              name: 'balance',
              status: 'changed',
              old: 'numeric(18,2)',
              new: 'numeric(19,2)',
            },
          ],
        },
        {
          id: 'tbl-transactions',
          kind: 'table',
          name: 'transactions',
          status: 'same',
          children: [
            { id: 'col-tx-id', kind: 'column', name: 'id', status: 'same', old: 'bigint', new: 'bigint' },
            { id: 'col-account-id', kind: 'column', name: 'account_id', status: 'same', old: 'bigint', new: 'bigint' },
            { id: 'col-amount', kind: 'column', name: 'amount', status: 'same', old: 'numeric(18,2)', new: 'numeric(18,2)' },
            { id: 'col-posted-at', kind: 'column', name: 'posted_at', status: 'same', old: 'timestamp', new: 'timestamp' },
            {
              id: 'col-memo',
              kind: 'column',
              name: 'memo',
              status: 'added',
              new: 'text',
            },
          ],
        },
        {
          id: 'tbl-archived',
          kind: 'table',
          name: 'archived_balances',
          status: 'missing',
        },
      ],
    },
  ],
  oldEncoding: 'LATIN1',
  newEncoding: 'UTF8',
  oldCollation: 'C',
  newCollation: 'en_US.utf8',
});

/**
 * A large inventory for the performance check: `tables` tables of five columns
 * each, a column type changed on every 50th table, one table missing in the new
 * database every 97th and one added every 101st.
 */
export const largeInventory = (tables: number): Inventory => {
  const rows: StructureRow[] = [
    { id: 'schema:bank', depth: 0, kind: 'schema', name: 'bank', status: 'same', old: 'present', new: 'present', expandable: true },
  ];
  for (let i = 0; i < tables; i += 1) {
    const name = `table_${i.toString().padStart(5, '0')}`;
    const id = `schema:bank/table:${name}`;
    const missing = i % 97 === 96;
    const added = i % 101 === 100;
    rows.push({
      id,
      depth: 1,
      kind: 'table',
      name,
      status: missing ? 'missing' : added ? 'added' : 'same',
      old: added ? undefined : '5 columns, 1 indexes, 1 constraints',
      new: missing ? undefined : '5 columns, 1 indexes, 1 constraints',
      expandable: !missing && !added,
    });
    if (missing || added) continue;
    for (const col of ['id', 'amount', 'posted_at', 'memo', 'branch']) {
      const changed = col === 'amount' && i % 50 === 0;
      rows.push({
        id: `${id}/column:${col}`,
        depth: 2,
        kind: 'column',
        name: col,
        status: changed ? 'changed' : 'same',
        old: col === 'amount' ? 'numeric(18,2)' : 'text',
        new: col === 'amount' ? (changed ? 'numeric(19,2)' : 'numeric(18,2)') : 'text',
        expandable: false,
      });
    }
  }
  return {
    schemas: [],
    rows,
    oldEncoding: 'LATIN1',
    newEncoding: 'UTF8',
    oldCollation: 'C',
    newCollation: 'en_US.utf8',
    oldServerVersion: '9.6.24',
    newServerVersion: '17.2',
  };
};

const makeMapping = (inventory: Inventory): Mapping => {
  const matches: MappingMatch[] = [];
  const needsAttention: MappingMatch[] = [];

  inventory.schemas.forEach((schema) => {
    schema.children?.forEach((table) => {
      if (table.status !== 'added') {
        const match: MappingMatch = {
          id: `m-${table.id}`,
          oldName: table.name,
          newName: table.name,
          confidence: 'exact',
          approved: true,
        };
        matches.push(match);
        if (table.status !== 'same') {
          needsAttention.push(match);
        }
      }
      table.children?.forEach((col) => {
        if (col.status !== 'added' && col.status !== 'missing') {
          const colMatch: MappingMatch = {
            id: `m-${table.id}-${col.id}`,
            oldName: `${table.name}.${col.name}`,
            newName: `${table.name}.${col.name}`,
            confidence: col.status === 'same' ? 'exact' : 'fuzzy',
            approved: col.status === 'same',
          };
          matches.push(colMatch);
          if (!colMatch.approved) {
            needsAttention.push(colMatch);
          }
        }
      });
    });
  });

  return {
    rules: ['identity mapping'],
    matches,
    needsAttention,
    ignored: [],
  };
};

const makeCheckPlan = (tableCount: number): CheckPlan => {
  const tables: TableCheck[] = [];
  for (let i = 0; i < tableCount; i += 1) {
    const hasAmount = i % 3 === 0;
    const hasBalance = i % 5 === 0;
    const level: TableCheck['level'] =
      i % 7 === 0 ? 'full-values' : i % 4 === 0 ? 'counts' : 'structure';
    const controls: string[] = [];
    if (hasAmount) controls.push('amount');
    if (hasBalance) controls.push('balance');
    tables.push({
      tableId: deterministicId('tbl', i),
      schemaName: 'demo_bank',
      tableName: `table_${i.toString().padStart(4, '0')}`,
      level,
      controlTotalColumns: controls.length ? controls : undefined,
    });
  }
  return {
    tables,
    presets: ['Full values everywhere', 'Counts for all, full values for selected'],
    estimateSecondsMin: Math.max(1, Math.round(tableCount * 0.5)),
    estimateSecondsMax: Math.max(2, Math.round(tableCount * 2.5)),
  };
};

const runStateForFixture = (fixture: string): TableRunState => {
  switch (fixture) {
    case 'fail':
    case 'widely-different':
    case 'needs-review':
    case 'warning':
      return 'done';
    case 'changed-during-check':
      return 'changed-during-check';
    case 'tool-fault':
      return 'failed';
    case 'pass':
    default:
      return 'done';
  }
};

const makeProgress = (runId: string, tableCount: number, fixture: string): RunProgress => {
  const overallState = fixture === 'cancelled' ? 'cancelled' : 'done';
  const tables: TableProgress[] = [];
  for (let i = 0; i < tableCount; i += 1) {
    const failed =
      fixture === 'fail' && i === 1
        ? true
        : fixture === 'widely-different' && i === 2
          ? true
          : fixture === 'changed-during-check' && i === 1
            ? true
            : fixture === 'tool-fault' && i === 0
              ? true
              : false;
    const state: TableRunState = failed
      ? runStateForFixture(fixture)
      : fixture === 'cancelled'
        ? i < tableCount / 2
          ? 'done'
          : 'cancelled'
        : 'done';
    tables.push({
      tableId: deterministicId('tbl', i),
      schemaName: 'demo_bank',
      tableName: `table_${i.toString().padStart(4, '0')}`,
      state,
      progressPercent: state === 'done' ? 100 : state === 'cancelled' ? 50 : 0,
      etaSeconds: 0,
    });
  }
  return {
    runId,
    overallState,
    tables,
    etaSecondsMin: 0,
    etaSecondsMax: 0,
  };
};

const drillDownSample = (kind: 'missing' | 'extra' | 'changed'): DrillDownRow => {
  if (kind === 'missing') {
    return {
      key: 'TX-1001',
      kind: 'missing',
      oldValues: { id: '1001', amount: '120.00' },
    };
  }
  if (kind === 'extra') {
    return {
      key: 'TX-9999',
      kind: 'extra',
      newValues: { id: '9999', amount: '55.00' },
    };
  }
  return {
    key: 'TX-2042',
    kind: 'changed',
    oldValues: { id: '2042', amount: '1000.00' },
    newValues: { id: '2042', amount: '999.99' },
    differingColumns: ['amount'],
  };
};

const makeIgnored = (count: number): IgnoreItem[] => {
  const reasons = [
    'Legacy archive table, not part of this migration',
    'Excluded by the review decision',
    'Out of scope for this sign-off',
  ];
  const items: IgnoreItem[] = [];
  for (let i = 0; i < count; i += 1) {
    items.push({
      id: deterministicId('ign', i),
      name: i === 0 ? 'archived_balances' : `ignored_table_${(i + 1).toString().padStart(4, '0')}`,
      reason: reasons[i % reasons.length],
    });
  }
  return items;
};

const mockConnection = (version: string, encoding: string): ConnectionFact => ({
  tls: 'verified',
  superuser: false,
  canWrite: false,
  serverVersion: version,
  serverEncoding: encoding,
});

/** Realistic recorded facts for one scanned table (counts only, never values). */
const makeFacts = (level: TableResult['level'], verdict: Verdict, failing: boolean): TableFacts | undefined => {
  if (level === 'structure') return undefined;
  const totals = [
    {
      column: 'amount',
      old: '1482930.50',
      new: failing ? '1482810.50' : '1482930.50',
    },
  ];
  const base: TableFacts = {
    keyColumns: [],
    columns: [],
    changeDetectionOld: 'available',
    changeDetectionNew: 'available',
    changedDuringScan: verdict === 'CHANGED_DURING_CHECK',
    controlTotals: totals,
  };
  if (level === 'counts') return base;
  return {
    ...base,
    keyColumns: ['id'],
    bucketsTotal: 256,
    spot: { state: 'agreed', compared: 200, differing: failing ? 3 : 0, note: '' },
    columns: [
      { column: 'id', rule: 'compared as an exact whole number' },
      { column: 'name', rule: 'compared exactly as text' },
      { column: 'amount', rule: 'compared as an exact decimal, digit for digit' },
      { column: 'posted_at', rule: 'compared as an exact date or time value' },
      { column: 'rate', rule: 'compared by its exact 8-byte binary form' },
    ],
  };
};

const makeResults = (fixture: string): Results => {
  const tableCount = fixture === 'scale' ? 1529 : 40;
  const coverage: CoverageSummary = {
    totalTables: tableCount,
    fullValuesTables: fixture === 'scale' ? 1400 : 12,
    countsTables: fixture === 'scale' ? 40 : 10,
    structureTables: fixture === 'scale' ? 80 : 15,
    ignoredTables: fixture === 'scale' ? 9 : 3,
  };

  const failures: FailureCard[] = [];
  const tableResults: TableResult[] = [];

  for (let i = 0; i < tableCount; i += 1) {
    let verdict: Verdict = 'PASS';
    let drillDown: DrillDownRow[] | undefined;

    if (fixture === 'fail' && i === 1) {
      verdict = 'FAIL';
      drillDown = [drillDownSample('missing'), drillDownSample('extra'), drillDownSample('changed')];
      failures.push({
        id: `fail-${i}`,
        tableId: deterministicId('tbl', i),
        schemaName: 'demo_bank',
        tableName: `table_${i.toString().padStart(4, '0')}`,
        level: 'full-values',
        verdict: 'FAIL',
        summary: '12 rows are missing and 3 rows differ',
        detail:
          'The new database has 12 fewer rows than the old one in the transaction history table, and 3 rows have changed values.',
      });
    } else if (fixture === 'widely-different' && i === 2) {
      verdict = 'WIDELY_DIFFERENT';
      drillDown = [drillDownSample('changed')];
      failures.push({
        id: `widediff-${i}`,
        tableId: deterministicId('tbl', i),
        schemaName: 'demo_bank',
        tableName: `table_${i.toString().padStart(4, '0')}`,
        level: 'full-values',
        verdict: 'WIDELY_DIFFERENT',
        summary: 'More than 10% of buckets differ',
        detail:
          'The table is widely different, so the full drill-down was skipped. A sample of differing rows is shown.',
      });
    } else if (fixture === 'needs-review' && i === 2) {
      verdict = 'NEEDS_REVIEW';
      failures.push({
        id: `review-${i}`,
        tableId: deterministicId('tbl', i),
        schemaName: 'demo_bank',
        tableName: `table_${i.toString().padStart(4, '0')}`,
        level: 'full-values',
        verdict: 'NEEDS_REVIEW',
        summary: 'Values matched but one column needs manual review',
        detail:
          'A column was compared through a printed form Dmux cannot prove is exact. A reviewer must confirm the values before sign-off.',
      });
    } else if (fixture === 'warning' && i === 3) {
      verdict = 'WARNING';
      failures.push({
        id: `warn-${i}`,
        tableId: deterministicId('tbl', i),
        schemaName: 'demo_bank',
        tableName: `table_${i.toString().padStart(4, '0')}`,
        level: 'counts',
        verdict: 'WARNING',
        summary: 'Counts match but no statistics on the new table',
        detail: 'The table passed the count check, but ANALYZE has not run on the new database.',
      });
    } else if (fixture === 'changed-during-check' && i === 1) {
      verdict = 'CHANGED_DURING_CHECK';
      failures.push({
        id: `cdc-${i}`,
        tableId: deterministicId('tbl', i),
        schemaName: 'demo_bank',
        tableName: `table_${i.toString().padStart(4, '0')}`,
        level: 'full-values',
        verdict: 'CHANGED_DURING_CHECK',
        summary: 'Rows were updated while the check ran',
        detail:
          'The insert, update or delete counters moved during the check. Run again when the table is not changing.',
      });
    } else if (fixture === 'tool-fault' && i === 0) {
      verdict = 'TOOL_FAULT';
      failures.push({
        id: `fault-${i}`,
        tableId: deterministicId('tbl', i),
        schemaName: 'demo_bank',
        tableName: `table_${i.toString().padStart(4, '0')}`,
        level: 'full-values',
        verdict: 'TOOL_FAULT',
        summary: 'The spot check disagreed with the fingerprint',
        detail:
          'This is reported as a tool fault, not a data result. Keep the run log and send it to the Dmux maintainers; do not sign off and do not read it as a PASS.',
      });
    }

    const card = failures.find((f) => f.tableId === deterministicId('tbl', i));
    const level: TableResult['level'] =
      card?.level ?? (i % 7 === 0 ? 'full-values' : i % 4 === 0 ? 'counts' : 'structure');
    const failing = verdict === 'FAIL' || verdict === 'WIDELY_DIFFERENT';
    const oldRows = 1000 + i * 37;
    const dataLevel = level !== 'structure';
    tableResults.push({
      tableId: deterministicId('tbl', i),
      schemaName: 'demo_bank',
      tableName: `table_${i.toString().padStart(4, '0')}`,
      verdict,
      level,
      checked: fixture !== 'cancelled',
      drillDown,
      columnNotes:
        i === 7
          ? [{ column: 'branch_code', kind: 'type_changed', note: 'type changed from integer to text' }]
          : undefined,
      oldRows: dataLevel ? oldRows : undefined,
      newRows: dataLevel ? (failing && i === 1 ? oldRows - 12 : oldRows) : undefined,
      differingBuckets: level === 'full-values' ? (failing ? 3 : 0) : undefined,
      missingBuckets: level === 'full-values' ? (failing ? 2 : 0) : undefined,
      extraBuckets: level === 'full-values' ? 0 : undefined,
      missingRows: failing ? 12 : undefined,
      extraRows: failing ? 0 : undefined,
      changedRows: failing ? 3 : undefined,
      reasons:
        verdict === 'PASS'
          ? level === 'structure'
            ? ['structure only: values were not compared']
            : []
          : failures.filter((f) => f.tableId === deterministicId('tbl', i)).map((f) => f.summary),
      facts: makeFacts(level, verdict, failing),
      ignoredColumns: i === 7 ? [{ id: 'demo_bank.table_0007.audit_note', name: 'audit_note', reason: 'free text, not migrated' }] : undefined,
    });
  }

  const verdict: Verdict =
    fixture === 'pass'
      ? 'PASS'
      : fixture === 'fail'
        ? 'FAIL'
        : fixture === 'widely-different'
          ? 'WIDELY_DIFFERENT'
          : fixture === 'needs-review'
            ? 'NEEDS_REVIEW'
            : fixture === 'changed-during-check'
              ? 'CHANGED_DURING_CHECK'
              : fixture === 'tool-fault'
                ? 'TOOL_FAULT'
                : fixture === 'warning'
                  ? 'WARNING'
                  : 'PASS';

  return {
    runId: `run-${fixture}`,
    verdict,
    coverage,
    ignored: makeIgnored(coverage.ignoredTables),
    failures,
    tableResults,
    optionsApplied: false,
    optionsRecorded: true,
    warnings: [],
    runFacts: {
      toolVersion: '1.0.0 (mock)',
      mappingHash: '3f2a9c1be0d84a7f6c55d2e1b9a0c4d87e6f1a2b3c4d5e6f708192a3b4c5d6e7',
      resultsSha256: '9b1de4c07a52f3e86d4b0a1c2e3f405162738495a6b7c8d9e0f1a2b3c4d5e6f7',
      oldConnection: mockConnection('9.6.24', 'LATIN1'),
      newConnection: mockConnection('17.2', 'UTF8'),
    },
  };
};

/** Stable fixtures used by unit and Playwright tests. */
export const fixtures = {
  pass: makeResults('pass'),
  fail: makeResults('fail'),
  warning: makeResults('warning'),
  needsReview: makeResults('needs-review'),
  changedDuringCheck: makeResults('changed-during-check'),
  widelyDifferent: makeResults('widely-different'),
  toolFault: makeResults('tool-fault'),
  scale: makeResults('scale'),
  cancelled: makeResults('cancelled'),
} as const;

export type FixtureName = keyof typeof fixtures;

export const fixtureNames: FixtureName[] = [
  'pass',
  'fail',
  'warning',
  'needsReview',
  'changedDuringCheck',
  'widelyDifferent',
  'toolFault',
  'scale',
  'cancelled',
];

export interface MockEngineOptions {
  /** Force a fixture for results; otherwise the engine uses the runId suffix. */
  forcedFixture?: FixtureName;
}

/**
 * Deterministic mock engine for browser and unit tests.
 *
 * The mock never touches a real database. It returns the same data for the same
 * inputs so tests are stable.
 */
export function createMockEngine(options: MockEngineOptions = {}): Engine {
  let currentRunId = 0;
  let superuserReason: string | null = null;
  // A mock keychain: the mock never touches a real OS keychain, but it models
  // the backend's "password lives in the keychain, not in the profile" shape so
  // `saveProfile` / `profileHasPassword` behave like the real engine.
  const savedPasswords = new Map<string, string>();

  return {
    setSuperuserOverride(override): void {
      superuserReason = override?.reason.trim() ? override.reason.trim() : null;
    },

    async testConnection(
      profile: ConnectionProfile,
      // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the mock ignores the one-time password
      _password?: string,
    ): Promise<ConnectionTestResult> {
      if (profile.host === 'error') {
        return { ok: false, error: 'Could not reach the database server.' };
      }
      if (profile.user === 'superuser') {
        // Like the real backend: refused unless the user overrode it with a reason.
        if (!superuserReason) {
          return { ok: false, error: 'superuser login refused by Dmux', superuserRefused: true };
        }
        return {
          ok: true,
          info: makeServerInfo({
            isSuperuser: true,
            canWrite: true,
            warning: `Superuser login allowed by you. Reason recorded in the results: ${superuserReason}`,
          }),
        };
      }
      if (profile.encryption === 'allow-plaintext') {
        return {
          ok: true,
          info: makeServerInfo({
            tls: { state: 'plaintext' },
            warning:
              'This connection is not encrypted. This choice will be recorded in the report.',
          }),
        };
      }
      if (profile.encryption === 'trust') {
        return {
          ok: true,
          info: makeServerInfo({
            tls: { state: 'encrypted', verified: false, fingerprint: 'a1:b2:c3:d4' },
          }),
        };
      }
      // A private-CA bundle only applies to 'verify'; the mock can't read a real
      // file, so it reports the connection as verified (the field still flows
      // through the ConnectionProfile shape unchanged).
      if (profile.caFile) {
        return {
          ok: true,
          info: makeServerInfo({
            tls: { state: 'encrypted', verified: true },
          }),
        };
      }
      return { ok: true, info: makeServerInfo() };
    },

    async saveProfile(
      profile: ConnectionProfile,
      password?: string,
    ): Promise<SaveProfileResult> {
      if (password) {
        savedPasswords.set(profile.id, password);
      }
      // The mock always has a working keychain, so there is no warning.
      return { profile };
    },

    async profileHasPassword(id: string): Promise<boolean> {
      return savedPasswords.has(id);
    },

    async compareStructure(
      oldProfile: ConnectionProfile,
      newProfile: ConnectionProfile,
    ): Promise<Inventory> {
      if (oldProfile.database === 'scale' || newProfile.database === 'scale') {
        return largeInventory(2000);
      }
      if (oldProfile.database === 'diff' || newProfile.database === 'diff') {
        return { ...diffInventory(), oldServerVersion: '9.6 (mock)', newServerVersion: '17.0 (mock)' };
      }
      return { ...baseInventory(), oldServerVersion: '17.0 (mock)', newServerVersion: '17.0 (mock)' };
    },

    async buildMapping(inventory: Inventory): Promise<Mapping> {
      return makeMapping(inventory);
    },

    async saveMapping(mapping: Mapping): Promise<{ hash: string; json: string }> {
      const json = JSON.stringify(mapping);
      // Simple deterministic hash for tests.
      let hash = 0;
      for (let i = 0; i < json.length; i += 1) {
        hash = (hash << 5) - hash + json.charCodeAt(i);
        hash = Math.trunc(hash);
      }
      return { hash: `mock-${Math.abs(hash).toString(16)}`, json };
    },

    async buildCheckPlan(): Promise<CheckPlan> {
      return makeCheckPlan(40);
    },

    async startRun(definition: RunDefinition): Promise<RunProgress> {
      currentRunId += 1;
      const fixture = options.forcedFixture ?? extractFixture(definition.oldProfile.database);
      return makeProgress(`run-${fixture}-${currentRunId}`, 40, fixture);
    },

    async pollRun(runId: string): Promise<RunProgress> {
      const fixture = options.forcedFixture ?? extractFixture(runId);
      return makeProgress(runId, 40, fixture);
    },

    async setRunPaused(runId: string, paused: boolean): Promise<RunProgress> {
      const progress = await this.pollRun(runId);
      progress.overallState = paused ? 'paused' : 'running';
      return progress;
    },

    async cancelRun(runId: string): Promise<RunProgress> {
      const progress = await this.pollRun(runId);
      progress.overallState = 'cancelled';
      progress.tables.forEach((t) => {
        if (t.state === 'queued' || t.state === 'running' || t.state === 'paused') {
          t.state = 'cancelled';
          t.progressPercent = 0;
        }
      });
      return progress;
    },

    async getResults(runId: string): Promise<Results> {
      const fixture = options.forcedFixture ?? extractFixture(runId);
      return fixtures[fixture] ?? fixtures.pass;
    },

    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the demo saves nothing
    async exportReport(_request: ReportRequest): Promise<ReportResult> {
      // The browser demo has no file system and no save dialog. Say so plainly
      // instead of pretending a file was saved somewhere.
      return { ok: false, demoOnly: true };
    },

    async getVersion(): Promise<string> {
      return version;
    },
  };
}

function extractFixture(input: string): FixtureName {
  for (const name of fixtureNames) {
    const kebab = name.replace(/([A-Z])/g, '-$1').toLowerCase();
    if (input.includes(kebab)) {
      return name;
    }
  }
  return 'pass';
}

export const mockEngine = createMockEngine();
