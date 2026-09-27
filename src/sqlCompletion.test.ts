import { describe, expect, it } from 'vitest';
import { EditorState } from '@codemirror/state';
import { CompletionContext } from '@codemirror/autocomplete';
import { PostgreSQL, sql } from '@codemirror/lang-sql';
import { tableCompletionSource } from './sqlCompletion';
import type { TableInfo } from './types';

const tables: TableInfo[] = [
  { oid: 1, schema: 'public', name: 'packages', kind: 'r', columns: ['pk', 'panel_service_id'], visible: true },
  { oid: 2, schema: 'public', name: 'quickbuy_rates', kind: 'r', columns: ['platform', 'service_type'], visible: true },
  { oid: 3, schema: 'archive', name: 'packages', kind: 'r', columns: ['old_id'], visible: false },
  { oid: 4, schema: 'public', name: 'Odd.Table', kind: 'r', columns: ['Mixed Case', 'a"b'], visible: true },
];
async function suggestions(marked: string, catalog = tables) {
  const pos = marked.indexOf('|');
  const state = EditorState.create({ doc: marked.replace('|', ''), extensions: [sql({ dialect: PostgreSQL })] });
  return await tableCompletionSource(catalog)(new CompletionContext(state, pos, true));
}
const labels = async (sql: string) => (await suggestions(sql))?.options.map(option => option.label) ?? [];

describe('schema autocomplete', () => {
  it('offers unqualified and schema-qualified tables before browsing any table', async () => {
    expect(await labels('SELECT * FROM pa|')).toContain('packages');
    expect(await labels('SELECT * FROM archive.pa|')).toEqual(['packages']);
  });
  it('offers columns from the current SELECT, UPDATE and INSERT', async () => {
    for (const query of ['SELECT pa| FROM packages', 'UPDATE packages SET pa|', 'INSERT INTO packages (pa|)', 'SELECT * FROM packages WHERE pa|']) {
      const options = await labels(query);
      expect(options, query).toContain('panel_service_id');
      expect(options, query).not.toContain('service_type');
    }
  });
  it('resolves aliases, joins, and explicit schema references', async () => {
    expect(await labels('SELECT p.| FROM packages p')).toEqual(['pk', 'panel_service_id']);
    expect(await labels('UPDATE packages AS p SET panel_service_id=1 WHERE p.|')).toEqual(['pk', 'panel_service_id']);
    expect(await labels('SELECT q.| FROM packages p JOIN quickbuy_rates q ON true')).toEqual(['platform', 'service_type']);
    expect(await labels('SELECT p.| FROM archive.packages p')).toEqual(['old_id']);
    expect(await labels('SELECT * FROM archive.packages WHERE |')).toContain('old_id');
  });
  it('limits column context to the current statement', async () => {
    const options = await labels('SELECT * FROM packages; SELECT | FROM quickbuy_rates');
    expect(options).toContain('service_type');
    expect(options).not.toContain('panel_service_id');
    expect(await labels('SELECT * FROM packages p; |')).not.toContain('panel_service_id');
  });
  it('quotes unusual names when inserting completions', async () => {
    const options = (await suggestions('SELECT | FROM "Odd.Table"'))!.options;
    expect(options.find(o => o.label === 'a"b')?.apply).toBe('"a""b"');
    expect(options.find(o => o.label === 'Mixed Case')?.apply).toBe('"Mixed Case"');
  });
  it('does not suggest identifiers in strings or comments', async () => {
    for (const query of ["SELECT 'pa|text' FROM packages", 'SELECT * FROM packages -- pa|', 'SELECT /* pa| */ 1']) {
      expect(await suggestions(query)).toBeNull();
    }
  });
  it('uses only the supplied connection catalog and picks up schema refreshes', async () => {
    const newTables = [{ ...tables[0], columns: ['new_column'] }];
    const options = (await suggestions('SELECT | FROM packages', newTables))!.options.map(o => o.label);
    expect(options).toContain('new_column');
    expect(options).not.toContain('panel_service_id');
    expect(options).not.toContain('quickbuy_rates');
  });
});
