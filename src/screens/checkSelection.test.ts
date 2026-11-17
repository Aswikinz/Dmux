import { describe, expect, it } from 'vitest';
import type { TableCheck } from '../api/contract';
import {
  MEGABYTE,
  formatSize,
  matchesPattern,
  selectByPattern,
  selectBySchema,
  selectBySize,
  unknownSizeCount,
} from './checkSelection';

const t = (schema: string, name: string, mb?: number): TableCheck => ({
  tableId: `${schema}.${name}`,
  schemaName: schema,
  tableName: name,
  level: 'full-values',
  sizeBytes: mb === undefined ? undefined : mb * MEGABYTE,
});

const tables = [
  t('public', 'acct_master', 500),
  t('public', 'ACCT_history', 10),
  t('archive', 'old_notes', 1),
  t('archive', 'unknown_size'),
];

describe('matchesPattern', () => {
  it('treats * as any run of characters and ignores case', () => {
    expect(matchesPattern('acct_*', 'ACCT_master')).toBe(true);
    expect(matchesPattern('*_notes', 'old_notes')).toBe(true);
    expect(matchesPattern('a*t', 'acct')).toBe(true);
    expect(matchesPattern('acct_*', 'my_acct_x')).toBe(false);
  });

  it('matches names that contain a pattern with no star', () => {
    expect(matchesPattern('notes', 'old_notes')).toBe(true);
    expect(matchesPattern('xyz', 'old_notes')).toBe(false);
  });

  it('takes every other character literally', () => {
    expect(matchesPattern('a.c', 'abc')).toBe(false);
    expect(matchesPattern('a.c', 'a.c')).toBe(true);
    expect(matchesPattern('(x)+', '(x)+')).toBe(true);
    expect(matchesPattern('[', 'a')).toBe(false);
  });

  it('matches nothing for an empty pattern', () => {
    expect(matchesPattern('', 'abc')).toBe(false);
    expect(matchesPattern('   ', 'abc')).toBe(false);
  });
});

describe('selection helpers', () => {
  it('selects by schema', () => {
    expect(selectBySchema(tables, 'archive')).toEqual(['archive.old_notes', 'archive.unknown_size']);
  });

  it('selects by name pattern, on the table name or schema.table', () => {
    expect(selectByPattern(tables, 'acct_*')).toEqual(['public.acct_master', 'public.ACCT_history']);
    expect(selectByPattern(tables, 'archive.*')).toEqual(['archive.old_notes', 'archive.unknown_size']);
  });

  it('selects by size, never picking a table whose size is unknown', () => {
    expect(selectBySize(tables, 'larger', 100)).toEqual(['public.acct_master']);
    expect(selectBySize(tables, 'smaller', 10)).toEqual(['archive.old_notes']);
    expect(selectBySize(tables, 'smaller', 1000)).not.toContain('archive.unknown_size');
    expect(unknownSizeCount(tables)).toBe(1);
  });

  it('formats sizes in plain words', () => {
    expect(formatSize(undefined)).toBe('—');
    expect(formatSize(500)).toBe('500 bytes');
    expect(formatSize(3 * MEGABYTE)).toBe('3 MB');
    expect(formatSize(1.5 * 1024 * MEGABYTE)).toBe('1.5 GB');
  });
});
