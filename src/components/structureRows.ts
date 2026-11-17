import type { SchemaDiffNode, StructureRow } from '../api/contract';

/** What a row with no recorded definition shows when the object exists. */
const PRESENT = 'present';

/**
 * Flatten the tree the older backends send into aligned rows, so the
 * side-by-side view has one code path.  A node that exists on a side shows its
 * recorded definition (or "present"); a missing or added node leaves the other
 * side empty, which the view draws as a gap.
 */
export function rowsFromNodes(nodes: SchemaDiffNode[], depth = 0): StructureRow[] {
  const out: StructureRow[] = [];
  for (const node of nodes) {
    const hasChildren = (node.children?.length ?? 0) > 0;
    out.push({
      id: node.id,
      depth,
      kind: node.kind,
      name: node.name,
      status: node.status,
      old: node.status === 'added' ? undefined : (node.old ?? PRESENT),
      new: node.status === 'missing' ? undefined : (node.new ?? PRESENT),
      expandable: hasChildren,
    });
    if (hasChildren) {
      out.push(...rowsFromNodes(node.children ?? [], depth + 1));
    }
  }
  return out;
}

export interface VisibleRow extends StructureRow {
  /** Rows below this one that differ (0 for a leaf). */
  problemsBelow: number;
  expanded: boolean;
}

export interface VisibleOptions {
  /** Rows the user flipped away from their default open/closed state. */
  toggled: ReadonlySet<string>;
  onlyProblems: boolean;
  query: string;
}

export interface VisibleResult {
  rows: VisibleRow[];
  /** Every row, whatever the filters. */
  total: number;
  /** Rows that differ between the two databases. */
  differences: number;
}

function matches(row: StructureRow, q: string): boolean {
  return (
    row.name.toLowerCase().includes(q) ||
    (row.old?.toLowerCase().includes(q) ?? false) ||
    (row.new?.toLowerCase().includes(q) ?? false)
  );
}

/**
 * The rows to draw, in tree order.  Linear in the number of rows, so thousands
 * of tables stay cheap.  A search keeps a row that matches, a row with a match
 * below it (so the path to it stays visible) and the rows under a match; "only
 * problems" keeps differing rows and the rows above them.  Schemas, and any
 * row with a difference below it, start open; everything else starts closed.
 */
export function visibleRows(rows: StructureRow[], opts: VisibleOptions): VisibleResult {
  const q = opts.query.trim().toLowerCase();
  const n = rows.length;
  const parent = new Array<number>(n).fill(-1);
  const stack: number[] = [];
  for (let i = 0; i < n; i += 1) {
    while (stack.length > 0 && rows[stack[stack.length - 1]].depth >= rows[i].depth) {
      stack.pop();
    }
    parent[i] = stack.length > 0 ? stack[stack.length - 1] : -1;
    stack.push(i);
  }

  const problemsBelow = new Array<number>(n).fill(0);
  const selfMatch = rows.map((r) => q !== '' && matches(r, q));
  // A row is kept when it passes both filters: it matches the search (itself or
  // through an ancestor that matches) and, with "only problems", it differs.
  const underMatch = new Array<boolean>(n).fill(false);
  const kept = new Array<boolean>(n).fill(false);
  for (let i = 0; i < n; i += 1) {
    const p = parent[i];
    underMatch[i] = p >= 0 && (selfMatch[p] || underMatch[p]);
    kept[i] =
      (q === '' || selfMatch[i] || underMatch[i]) && (!opts.onlyProblems || rows[i].status !== 'same');
  }
  const keptBelow = new Array<boolean>(n).fill(false);
  let differences = 0;
  for (let i = n - 1; i >= 0; i -= 1) {
    const own = rows[i].status !== 'same' ? 1 : 0;
    differences += own;
    const p = parent[i];
    if (p >= 0) {
      problemsBelow[p] += problemsBelow[i] + own;
      if (kept[i] || keptBelow[i]) {
        keptBelow[p] = true;
      }
    }
  }

  const out: VisibleRow[] = [];
  // Per ancestor on the current path: is it open?
  const open: boolean[] = [];
  const pathDepth: number[] = [];
  for (let i = 0; i < n; i += 1) {
    const row = rows[i];
    while (pathDepth.length > 0 && pathDepth[pathDepth.length - 1] >= row.depth) {
      pathDepth.pop();
      open.pop();
    }
    const defaultOpen = q !== '' ? keptBelow[i] : row.depth === 0 || problemsBelow[i] > 0;
    const expanded = row.expandable && (opts.toggled.has(row.id) ? !defaultOpen : defaultOpen);
    const hiddenByCollapse = open.some((o) => !o);
    pathDepth.push(row.depth);
    open.push(expanded);
    if (hiddenByCollapse || !(kept[i] || keptBelow[i])) {
      continue;
    }
    out.push({ ...row, problemsBelow: problemsBelow[i], expanded });
  }
  return { rows: out, total: n, differences };
}
