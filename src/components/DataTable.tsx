import {
  useReactTable,
  getCoreRowModel,
  flexRender,
  type ColumnDef,
  type RowSelectionState,
} from '@tanstack/react-table';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useRef, useState } from 'react';
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

  const filteredData = filterFn
    ? data.filter((row) => filterFn(row, filter.toLowerCase()))
    : data.filter((row) =>
        Object.values(row)
          .filter((v): v is string => typeof v === 'string')
          .some((v) => v.toLowerCase().includes(filter.toLowerCase())),
      );

  const table = useReactTable({
    data: filteredData,
    columns,
    state: enableSelection ? { rowSelection } : undefined,
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
          className="w-full rounded-lg border border-slate-300 py-2 pl-9 pr-3 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
        />
      </div>
      <div
        ref={parentRef}
        className="overflow-auto rounded-lg border border-slate-200 dark:border-slate-700"
        style={{ maxHeight: '60vh' }}
        tabIndex={0}
        role="region"
        aria-label="Table results"
      >
        <table className="w-full border-collapse text-sm">
          <thead className="sticky top-0 z-10 bg-slate-50 dark:bg-slate-800">
            {table.getHeaderGroups().map((headerGroup) => (
              <tr key={headerGroup.id}>
                {headerGroup.headers.map((header) => (
                  <th
                    key={header.id}
                    className="border-b border-slate-200 px-3 py-2 text-left font-medium text-slate-700 dark:border-slate-700 dark:text-slate-300"
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
                        className="flex"
                      >
                        {row.getVisibleCells().map((cell) => (
                          <div
                            key={cell.id}
                            className="flex-1 border-b border-slate-100 px-3 py-2 dark:border-slate-800"
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
