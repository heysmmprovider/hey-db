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

export type ExportFormat = 'csv' | 'json' | 'tsv';
export function exportData(result: QueryResult, format: ExportFormat): { text: string; mime: string } {
  if (format === 'json') {
    // Keep PostgreSQL values as strings to preserve numeric precision. Assign
    // unique keys to duplicate column labels without overwriting another column.
    const used = new Set<string>();
    const reserved = new Set(result.columns.map(column => column.name));
    const keys = result.columns.map(column => {
      let key = column.name; let suffix = 2;
      while (used.has(key) || (key !== column.name && reserved.has(key))) {
        key = `${column.name}_${suffix++}`;
      }
      used.add(key); return key;
    });
    return { text: JSON.stringify(result.rows.map(row => Object.fromEntries(keys.map((key, index) => [key, row[index]]))), null, 2), mime: 'application/json;charset=utf-8' };
  }
  if (format === 'csv') return { text: toCsv(result), mime: 'text/csv;charset=utf-8' };
  const field = (value: string | null) => {
    if (value === null) return '';
    const safe = /^[\s]*[=+@-]/.test(value) || /^[\t\r]/.test(value) ? `'${value}` : value;
    return safe === '' || /[\t\r\n"]/.test(safe) ? `"${safe.replaceAll('"', '""')}"` : safe;
  };
  return { text: [result.columns.map(c => field(c.name)).join('\t'), ...result.rows.map(row => row.map(field).join('\t'))].join('\r\n'), mime: 'text/tab-separated-values;charset=utf-8' };
}
