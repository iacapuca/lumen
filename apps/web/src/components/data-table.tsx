// Generic table wrapper — the standard shadcn/TanStack Table pattern (column
// defs + useReactTable + flexRender over the existing shadcn Table
// primitives). Reuse this for any future tabular data in the control plane
// (e.g. the Usage page) rather than hand-rolling <TableRow> maps again.
import { useState } from 'react'
import {
  type Column,
  type ColumnDef,
  type SortingState,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { ArrowUpDown } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'

export function DataTable<TData, TValue>({
  columns,
  data,
  emptyMessage = 'No results.',
}: {
  columns: ColumnDef<TData, TValue>[]
  data: TData[]
  emptyMessage?: string
}) {
  const [sorting, setSorting] = useState<SortingState>([])

  const table = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  })

  return (
    <Table>
      <TableHeader>
        {table.getHeaderGroups().map((headerGroup) => (
          <TableRow key={headerGroup.id}>
            {headerGroup.headers.map((header) => (
              <TableHead key={header.id}>
                {header.isPlaceholder
                  ? null
                  : flexRender(header.column.columnDef.header, header.getContext())}
              </TableHead>
            ))}
          </TableRow>
        ))}
      </TableHeader>
      <TableBody>
        {table.getRowModel().rows.length ? (
          table.getRowModel().rows.map((row) => (
            <TableRow key={row.id}>
              {row.getVisibleCells().map((cell) => (
                <TableCell key={cell.id}>
                  {flexRender(cell.column.columnDef.cell, cell.getContext())}
                </TableCell>
              ))}
            </TableRow>
          ))
        ) : (
          <TableRow>
            <TableCell colSpan={columns.length} className="h-24 text-center text-muted-foreground">
              {emptyMessage}
            </TableCell>
          </TableRow>
        )}
      </TableBody>
    </Table>
  )
}

/** Drop into a `ColumnDef.header` to make that column clickable-to-sort. */
export function SortableHeader<TData>({ column, label }: { column: Column<TData, unknown>; label: string }) {
  return (
    <Button variant="ghost" size="sm" className="-ml-3" onClick={() => column.toggleSorting(column.getIsSorted() === 'asc')}>
      {label}
      <ArrowUpDown className="ml-2 size-3.5" />
    </Button>
  )
}
