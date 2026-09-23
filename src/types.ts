export type Profile = { id: string; name: string; host: string; port: number; database: string; username: string; tls: 'verify-full' | 'disable'; readOnly: boolean; rememberPassword: boolean };
export type ResultColumn = { name: string; dataType: string; editable: boolean; primaryKey: boolean };
export type QueryResult = { id: string; columns: ResultColumn[]; rows: (string | null)[][]; affectedRows: number; elapsedMs: number; truncated: boolean; readOnlyReason: string | null; table: string | null };
export type CellEdit = { row: number; column: number; value: string | null };
export type PlannedUpdate = { sql: string; parameters: (string | null)[]; row: number };
export type TableInfo = { oid: number; schema: string; name: string; kind: string };
export type TableDetails = { columns: { name: string; dataType: string; nullable: boolean; defaultValue: string | null; primaryKey: boolean }[]; indexes: { name: string; definition: string }[] };
