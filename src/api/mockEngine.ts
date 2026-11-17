import type { Engine } from './engine';
import type {
  CheckPlan,
  ConnectionProfile,
  ConnectionTestResult,
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
  ServerInfo,
  TableCheck,
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

  if (inventory.oldEncoding !== inventory.newEncoding) {
    needsAttention.push({
      id: 'm-encoding',
      oldName: `encoding ${inventory.oldEncoding}`,
      newName: `encoding ${inventory.newEncoding}`,
      confidence: 'fuzzy',
      approved: false,
    });
  }

  return {
    rules: ['identity mapping', 'encoding review required'],
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

    tableResults.push({
      tableId: deterministicId('tbl', i),
      schemaName: 'demo_bank',
      tableName: `table_${i.toString().padStart(4, '0')}`,
      verdict,
      level: i % 7 === 0 ? 'full-values' : i % 4 === 0 ? 'counts' : 'structure',
      checked: fixture !== 'cancelled',
      drillDown,
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

  return {
    async testConnection(profile: ConnectionProfile): Promise<ConnectionTestResult> {
      if (profile.host === 'error') {
        return { ok: false, error: 'Could not reach the database server.' };
      }
      if (profile.user === 'superuser') {
        return {
          ok: true,
          info: makeServerInfo({
            isSuperuser: true,
            canWrite: true,
            warning:
              'This login is a superuser. Dmux refuses superuser logins by default because a read-only transaction can still do harm.',
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
      return { ok: true, info: makeServerInfo() };
    },

    async compareStructure(
      oldProfile: ConnectionProfile,
      newProfile: ConnectionProfile,
    ): Promise<Inventory> {
      if (oldProfile.database === 'diff' || newProfile.database === 'diff') {
        return diffInventory();
      }
      return baseInventory();
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
        hash |= 0;
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

    async exportReport(request: ReportRequest): Promise<ReportResult> {
      if (request.format === 'pdf') {
        return { ok: true, filePath: `${request.filePath}/report.pdf` };
      }
      return { ok: true, filePath: `${request.filePath}/report.xlsx` };
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
