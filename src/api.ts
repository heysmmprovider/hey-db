import { invoke, isTauri } from '@tauri-apps/api/core';
import type { CellEdit, PlannedUpdate, Profile, QueryResult, TableDetails, TableInfo } from './types';
import { DEFAULT_SQL, quoteIdentifier } from './data';
export const desktop = isTauri();
export const DEMO_ID = 'demo';
const demoRows: (string | null)[][] = [
  ['1001', 'Desk lamp', 'active', '42'], ['1002', 'Notebook', 'active', '128'], ['1003', 'Monitor stand', 'active', '16'],
  ['1004', 'Pen tray', 'active', '73'], ['1005', 'Cable organizer', 'active', '95'], ['1006', 'Desk mat', 'active', '28'],
  ['1007', 'Bookend', 'active', '54'], ['1008', 'Pencil cup', 'active', '31'], ['1009', 'Task light', 'active', '19'], ['1010', 'Desk shelf', 'active', '12'],
];
const demoColumns = [{ name: 'id', dataType: 'int4', primaryKey: true, editable: false }, { name: 'name', dataType: 'text', primaryKey: false, editable: true }, { name: 'status', dataType: 'text', primaryKey: false, editable: true }, { name: 'stock', dataType: 'int4', primaryKey: false, editable: true }];
let demoSnapshot: QueryResult | null = null;
export const demoProfile: Profile = { id: DEMO_ID, name: 'Demo store', host: 'Sample data', port: 5432, database: 'demo_store', username: 'demo', tls: 'disable', readOnly: false, rememberPassword: false };
export const api = {
  profiles: () => desktop ? invoke<Profile[]>('list_profiles') : Promise.resolve([]),
  saveProfile: (profile: Profile, password?: string) => invoke<void>('save_profile', { profile, password }),
  deleteProfile: (id: string) => invoke<void>('delete_profile', { id }),
  connect: (profile: Profile, password?: string) => invoke<void>('connect', { profile, password }),
  disconnect: (connectionId: string) => connectionId === DEMO_ID ? Promise.resolve() : invoke<void>('disconnect', { connectionId }),
  tables: (connectionId: string) => connectionId === DEMO_ID ? Promise.resolve([{ oid: 1, schema: 'public', name: 'products', kind: 'r' }]) : invoke<TableInfo[]>('list_tables', { connectionId }),
  details: (connectionId: string, oid: number) => connectionId === DEMO_ID ? Promise.resolve<TableDetails>({ columns: demoColumns.map(c => ({ name: c.name, dataType: c.dataType === 'int4' ? 'integer' : 'text', nullable: false, defaultValue: c.name === 'stock' ? '0' : null, primaryKey: c.primaryKey })), indexes: [{ name: 'products_pkey', definition: 'CREATE UNIQUE INDEX products_pkey ON public.products USING btree (id)' }] }) : invoke<TableDetails>('table_details', { connectionId, oid }),
  query: async (connectionId: string, sql: string, operationId: string): Promise<QueryResult> => {
    if (connectionId !== DEMO_ID) return invoke<QueryResult>('run_query', { connectionId, sql, operationId });
    const normalized = sql.trim().replace(/;$/, '').replace(/\s+/g, ' ').toLowerCase();
    const accepted = [DEFAULT_SQL, 'SELECT * FROM "public"."products" LIMIT 1000;', 'SELECT * FROM "public"."products" ORDER BY "id" LIMIT 1000;'].map(s => s.trim().replace(/;$/, '').replace(/\s+/g, ' ').toLowerCase());
    if (!accepted.includes(normalized)) throw new Error('The demo supports its sample query only. Connect to PostgreSQL to run your own SQL.');
    demoSnapshot = { id: crypto.randomUUID(), columns: demoColumns, rows: demoRows.filter(r => normalized.includes("'active'") ? r[2] === 'active' : true).map(r => [...r]), affectedRows: 0, elapsedMs: 0, truncated: false, readOnlyReason: null, table: '"public"."products"' };
    return structuredClone(demoSnapshot);
  },
  cancel: (connectionId: string, operationId: string) => invoke<void>('cancel_query', { connectionId, operationId }),
  preview: async (connectionId: string, resultId: string, edits: CellEdit[]): Promise<PlannedUpdate[]> => {
    if (connectionId !== DEMO_ID) return invoke<PlannedUpdate[]>('preview_edits', { connectionId, resultId, edits });
    if (!demoSnapshot || resultId !== demoSnapshot.id) throw new Error('Run the sample query again.');
    return [...new Set(edits.map(e => e.row))].map(row => {
      const cells = edits.filter(e => e.row === row);
      const parameters = cells.map(e => e.value);
      parameters.push(demoSnapshot!.rows[row][0]);
      const keyIndex = parameters.length;
      const conditions = cells.map(e => { parameters.push(demoSnapshot!.rows[row][e.column]); return `${quoteIdentifier(demoColumns[e.column].name)}::text IS NOT DISTINCT FROM $${parameters.length}`; });
      return { row, sql: `UPDATE ONLY "public"."products"\nSET ${cells.map((e, i) => `${quoteIdentifier(demoColumns[e.column].name)} = $${i + 1}`).join(', ')}\nWHERE "id" = $${keyIndex}\n  AND ${conditions.join('\n  AND ')};`, parameters };
    });
  },
  apply: async (connectionId: string, resultId: string, edits: CellEdit[]): Promise<number> => {
    if (connectionId !== DEMO_ID) return invoke<number>('apply_edits', { connectionId, resultId, edits });
    if (!demoSnapshot || resultId !== demoSnapshot.id) throw new Error('Run the sample query again.');
    for (const e of edits) {
      if (e.value === null) throw new Error('The sample columns do not allow NULL. No changes applied.');
      if (e.column === 3 && (!/^-?\d+$/.test(e.value) || Number(e.value) < -2147483648 || Number(e.value) > 2147483647)) throw new Error('Stock must be a valid integer. No changes applied.');
    }
    for (const e of edits) { const source = demoRows.find(row => row[0] === demoSnapshot!.rows[e.row][0])!; source[e.column] = e.value; }
    demoSnapshot = null;
    return new Set(edits.map(e => e.row)).size;
  },
};
