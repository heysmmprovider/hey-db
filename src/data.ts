import type { CellEdit, QueryResult } from './types';
export const DEFAULT_SQL = "SELECT id, name, status, stock\nFROM public.products\nWHERE status = 'active'\nORDER BY id\nLIMIT 100;";
export const quoteIdentifier = (value: string) => `"${value.replaceAll('"', '""')}"`;
export const cellKey = (row: number, column: number) => `${row}:${column}`;
export function stageEdit(current: CellEdit[], result: QueryResult, next: CellEdit): CellEdit[] {
  if (!result.columns[next.column]?.editable || !result.rows[next.row]) return current;
  const rest = current.filter(e => e.row !== next.row || e.column !== next.column);
  return result.rows[next.row][next.column] === next.value ? rest : [...rest, next];
}
export function toCsv(result: QueryResult): string {
  // Neutralize spreadsheet formula execution. NULL is an unquoted empty field;
  // empty strings are quoted, so a CSV-aware importer can distinguish them.
  const field = (value: string | null) => {
    if (value === null) return '';
    const safe = /^[\s]*[=+@-]/.test(value) || /^[\t\r]/.test(value) ? `'${value}` : value;
    return `"${safe.replaceAll('"', '""')}"`;
  };
  return [result.columns.map(c => field(c.name)).join(','), ...result.rows.map(row => row.map(field).join(','))].join('\r\n');
}
