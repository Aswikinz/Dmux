import type { TableCheck } from '../api/contract';

/** One megabyte, as the size filter counts it. */
export const MEGABYTE = 1024 * 1024;

/**
 * Whether `text` matches a simple name pattern.  `*` stands for any run of
 * characters (including none); everything else is literal; case does not
 * matter.  A pattern without `*` matches names that contain it.
 */
export function matchesPattern(pattern: string, text: string): boolean {
  const p = pattern.trim().toLowerCase();
  if (!p) return false;
  const parts = p.split('*').map((part) => part.replace(/[.+?^${}()|[\]\\]/g, '\\$&'));
  const source = p.includes('*') ? `^${parts.join('.*')}$` : parts[0];
  return new RegExp(source).test(text.toLowerCase());
}

/** Ids of the tables in `schema`. */
export function selectBySchema(tables: TableCheck[], schema: string): string[] {
  return tables.filter((t) => t.schemaName === schema).map((t) => t.tableId);
}

/** Ids of the tables whose name, or `schema.name`, matches `pattern`. */
export function selectByPattern(tables: TableCheck[], pattern: string): string[] {
  return tables
    .filter(
      (t) =>
        matchesPattern(pattern, t.tableName) ||
        matchesPattern(pattern, `${t.schemaName}.${t.tableName}`),
    )
    .map((t) => t.tableId);
}

/**
 * Ids of the tables larger or smaller than `megabytes`.  A table whose size is
 * not known is never selected by size: guessing would hide it from the user.
 */
export function selectBySize(
  tables: TableCheck[],
  direction: 'larger' | 'smaller',
  megabytes: number,
): string[] {
  const limit = megabytes * MEGABYTE;
  return tables
    .filter((t) => {
      if (t.sizeBytes === undefined) return false;
      return direction === 'larger' ? t.sizeBytes > limit : t.sizeBytes < limit;
    })
    .map((t) => t.tableId);
}

/** Tables whose size the engine did not report. */
export function unknownSizeCount(tables: TableCheck[]): number {
  return tables.filter((t) => t.sizeBytes === undefined).length;
}

/** A size in plain words: "3 MB", "1.5 GB". */
export function formatSize(bytes: number | undefined): string {
  if (bytes === undefined) return '—';
  if (bytes < 1024) return `${bytes} bytes`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 || Number.isInteger(value) ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}
