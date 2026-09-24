import { describe, expect, it } from 'vitest';
import { quoteIdentifier, stageEdit, toCsv, exportData } from './data';
import type { QueryResult } from './types';
const result: QueryResult = { id: 'sample', columns: [{ name: 'id', dataType: 'int8', primaryKey: true, editable: false }, { name: 'name', dataType: 'text', primaryKey: false, editable: true }], rows: [['9007199254740993', 'Monitor stand']], affectedRows: 0, elapsedMs: 0, truncated: false, readOnlyReason: null, table: 'products' };
describe('pending edits', () => {
  it('reverts changes without turning NULL into an empty string', () => {
    let edits = stageEdit([], result, { row: 0, column: 1, value: null });
    expect(edits[0].value).toBeNull();
    edits = stageEdit(edits, result, { row: 0, column: 1, value: '' });
    expect(edits).toHaveLength(1); expect(edits[0].value).toBe('');
    expect(stageEdit(edits, result, { row: 0, column: 1, value: 'Monitor stand' })).toEqual([]);
  });
  it('cannot edit keys and keeps large integers as exact strings', () => {
    expect(stageEdit([], result, { row: 0, column: 0, value: '2' })).toEqual([]);
    expect(toCsv(result)).toContain('9007199254740993');
  });
});
describe('CSV exports', () => {
  it('escapes quotes, commas, line breaks and spreadsheet formulas', () => {
    const csv = toCsv({ ...result, rows: [['1', 'a,"b"\nc'], ['2', '=HYPERLINK("https://example.invalid")'], ['3', null], ['4', '']] });
    expect(csv).toContain('"a,""b""\nc"'); expect(csv).toContain('"\'=HYPERLINK'); expect(csv).toContain('"3",\r\n"4",""');
  });
  it('quotes unusual PostgreSQL identifiers', () => { expect(quoteIdentifier('Odd" table')).toBe('"Odd"" table"'); });
});

describe('export formats', () => {
  it('preserves precision, NULL, and empty strings in JSON', () => {
    const data = exportData({ ...result, rows: [['9007199254740993', null], ['2', '']] }, 'json');
    expect(JSON.parse(data.text)).toEqual([{ id: '9007199254740993', name: null }, { id: '2', name: '' }]);
  });
  it('preserves duplicate column names without colliding with existing labels', () => {
    const columns = ['name', 'name', 'name_2', '__proto__', 'name', 'name_3'].map(name => ({ ...result.columns[1], name }));
    const data = exportData({ ...result, columns, rows: [['a', 'b', 'c', 'd', 'e', 'f']] }, 'json');
    expect(Object.entries(JSON.parse(data.text)[0])).toEqual([['name', 'a'], ['name_4', 'b'], ['name_2', 'c'], ['__proto__', 'd'], ['name_5', 'e'], ['name_3', 'f']]);
  });
  it('escapes TSV delimiters and formulas while distinguishing NULL and empty text', () => {
    const data = exportData({ ...result, rows: [['1', 'a\tb\nc"d'], ['2', '=SUM(A1)'], ['3', null], ['4', '']] }, 'tsv');
    expect(data.text).toBe('id\tname\r\n1\t"a\tb\nc""d"\r\n2\t\'=SUM(A1)\r\n3\t\r\n4\t""');
  });
  it('uses the existing CSV serialization', () => { expect(exportData(result, 'csv').text).toBe(toCsv(result)); });
});
