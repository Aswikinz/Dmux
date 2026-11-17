import {
  useReactTable,
  getCoreRowModel,
  flexRender,
  type ColumnDef,
  type RowSelectionState,
} from '@tanstack/react-table';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useMemo, useRef, useState } from 'react';
import { cn } from '../design/cn';
import { Search } from 'lucide-react';

interface DataTableProps<T extends object> {
  data: T[];
  columns: ColumnDef<T>[];
  rowHeight?: number;
  enableSelection?: boolean;
  onRowSelectionChange?: (rows: T[]) => void;
  className?: string;
  filterPlaceholder?: string;
  filterFn?: (row: T, query: string) => boolean;
  getRowId?: (row: T, index: number) => string;
}

export function DataTable<T extends object>({
  data,
  columns,
  rowHeight = 40,
  enableSelection = false,
  onRowSelectionChange,
  className,
  filterPlaceholder = 'Search...',
  filterFn,
  getRowId = (row, index) => (row as Record<string, unknown>).id as string ?? String(index),
}: DataTableProps<T>) {
  const [filter, setFilter] = useState('');
  const [rowSelection, setRowSelection] = useState<RowSelectionState>({});
  const parentRef = useRef<HTMLDivElement>(null);

  // Memoised: react-table treats a new `data` array as new input and resets its
  // state, so a fresh array on every render re-rendered the table forever once
  // the real virtualizer was involved (clicking any button on Results froze the
  // app).
  const filteredData = useMemo(
    () =>
      filterFn
        ? data.filter((row) => filterFn(row, filter.toLowerCase()))
        : data.filter((row) =>
            Object.values(row)
              .filter((v): v is string => typeof v === 'string')
              .some((v) => v.toLowerCase().includes(filter.toLowerCase())),
          ),
    [data, filter, filterFn],
  );

  const table = useReactTable({
    data: filteredData,
    columns,
    state: enableSelection ? { rowSelection } : undefined,
    // There is no paging here.  Without this, react-table resets the page index
    // whenever `data` changes identity, and `filteredData` is a new array on
    // every render, so any parent re-render would loop forever.
    autoResetPageIndex: false,
    enableRowSelection: enableSelection,
    onRowSelectionChange: (updater) => {
      const next = typeof updater === 'function' ? updater(rowSelection) : updater;
      setRowSelection(next);
      if (onRowSelectionChange) {
        const selected = table.getSelectedRowModel().rows.map((r) => r.original);
        onRowSelectionChange(selected);
      }
    },
    getCoreRowModel: getCoreRowModel(),
    getRowId,
  });

  const virtualizer = useVirtualizer({
    count: table.getRowModel().rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 10,
  });

  return (
    <div className={cn('flex flex-col gap-2', className)}>
      <div className="relative">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-400" aria-hidden="true" />
        <input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={filterPlaceholder}
          aria-label={filterPlaceholder}
          className="h-9 w-full rounded-[10px] border border-black/[0.1] bg-black/[0.03] pl-9 pr-3 text-sm transition-[border-color,box-shadow,background-color] duration-150 placeholder:text-slate-400 focus:border-accent-500 focus:bg-white focus:outline-none focus:ring-4 focus:ring-accent-500/20 dark:border-white/10 dark:bg-white/[0.06] dark:text-slate-100"
        />
      </div>
      <div
        ref={parentRef}
        className="overflow-auto rounded-xl border border-black/[0.06] dark:border-white/[0.08]"
        style={{ maxHeight: '60vh' }}
        tabIndex={0}
        role="region"
        aria-label="Table results"
      >
        <table className="w-full border-collapse text-sm">
          <thead className="sticky top-0 z-10 bg-[#fafafc]/95 backdrop-blur dark:bg-[#2a2a2d]/95">
            {table.getHeaderGroups().map((headerGroup) => (
              <tr key={headerGroup.id} className="flex">
                {headerGroup.headers.map((header) => (
                  <th
                    key={header.id}
                    style={{ flexBasis: header.column.getSize() }}
                    className="flex-1 border-b border-black/[0.06] px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-[0.05em] text-ink-secondary dark:border-white/[0.08] dark:text-slate-400"
                  >
                    {header.isPlaceholder
                      ? null
                      : flexRender(header.column.columnDef.header, header.getContext())}
                  </th>
                ))}
              </tr>
            ))}
          </thead>
          <tbody>
            <tr>
              <td
                colSpan={columns.length}
                style={{ height: `${virtualizer.getTotalSize()}px` }}
                className="p-0"
              >
                <div style={{ position: 'relative', height: `${virtualizer.getTotalSize()}px` }}>
                  {virtualizer.getVirtualItems().map((virtualRow) => {
                    const row = table.getRowModel().rows[virtualRow.index];
                    return (
                      <div
                        key={row.id}
                        data-index={virtualRow.index}
                        ref={virtualizer.measureElement}
                        style={{
                          position: 'absolute',
                          top: 0,
                          left: 0,
                          width: '100%',
                          transform: `translateY(${virtualRow.start}px)`,
                        }}
                        className="flex transition-colors hover:bg-accent-50/50 dark:hover:bg-white/[0.03]"
                      >
                        {row.getVisibleCells().map((cell) => (
                          <div
                            key={cell.id}
                            className="flex flex-1 items-center border-b border-black/[0.04] px-3 py-2 dark:border-white/[0.05]"
                            style={{ flexBasis: cell.column.getSize() }}
                          >
                            {flexRender(cell.column.columnDef.cell, cell.getContext())}
                          </div>
                        ))}
                      </div>
                    );
                  })}
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}
