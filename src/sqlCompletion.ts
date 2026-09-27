import { PostgreSQL, schemaCompletionSource, type SQLNamespace } from '@codemirror/lang-sql';
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import type { Completion, CompletionContext, CompletionSource } from '@codemirror/autocomplete';
import type { SyntaxNode } from '@lezer/common';
import type { TableInfo } from './types';
import { quoteIdentifier } from './data';

const identifier = /"(?:[^"]|"")*"|[\w$\u0080-\uffff]+/g;
const namespaceKey = (name: string) => name.replaceAll('.', '\\.');
const keywords = new Set((PostgreSQL.spec.keywords ?? '').split(' '));
const completion = (label: string, type: string, detail?: string): Completion => ({
  label, type, detail,
  apply: /^[a-z_][a-z_\d]*$/.test(label) && !keywords.has(label) ? label : quoteIdentifier(label),
});

function path(context: CompletionContext, node: SyntaxNode): string[] {
  return (context.state.sliceDoc(node.from, node.to).match(identifier) ?? [])
    .map(name => name.startsWith('"') ? name.slice(1, -1).replaceAll('""', '"') : name.toLowerCase());
}

// The language package handles schema paths and SELECT aliases. Add columns from
// the current statement and aliases for UPDATE/INSERT as well, without requiring
// a table to be opened in the explorer first.
export function tableCompletionSource(tables: TableInfo[]): CompletionSource {
  const schema: Record<string, SQLNamespace> = Object.create(null);
  const qualified = new Map<string, TableInfo>();
  const visible = new Map<string, TableInfo>();
  const columns = (table: TableInfo) => table.columns.map(name => completion(name, 'property', `${table.schema}.${table.name}`));
  for (const table of tables) {
    qualified.set(JSON.stringify([table.schema, table.name]), table);
    if (table.visible) visible.set(table.name, table);
    const scope = namespaceKey(table.schema);
    const children = (schema[scope] ??= Object.create(null)) as Record<string, SQLNamespace>;
    children[namespaceKey(table.name)] = { self: completion(table.name, 'type', table.schema), children: columns(table) };
  }
  for (const table of visible.values()) {
    // Keep an identically named schema accessible through qualification.
    schema[namespaceKey(table.name)] ??= { self: completion(table.name, 'type', table.schema), children: columns(table) };
  }
  const base = schemaCompletionSource({ dialect: PostgreSQL, schema });
  return context => {
    const tree = ensureSyntaxTree(context.state, context.state.doc.length, 50) ?? syntaxTree(context.state);
    const anchor = context.state.sliceDoc(0, context.pos).trimEnd().length;
    let node: SyntaxNode | null = tree.resolveInner(anchor, -1);
    if (/Comment|String/.test(node.name)) return null;
    const afterStatement = node.name === ';';
    if (afterStatement) node = null;
    while (node && node.name !== 'Statement') node = node.parent;
    const local: Record<string, SQLNamespace> = Object.create(null);
    const fields: Completion[] = [];
    let inFrom = false;
    for (let scan = node?.firstChild; scan; scan = scan.nextSibling) {
      const text = context.state.sliceDoc(scan.from, scan.to).toLowerCase();
      if (scan.name === 'Keyword' && /^(where|set|values|returning|group|order|having|limit|union|except|intersect)$/.test(text)) inFrom = false;
      const startsTable = scan.name === 'Keyword' && /^(from|join|update|into|table)$/.test(text);
      if (text === 'from' && scan.name === 'Keyword') inFrom = true;
      if (!startsTable && !(inFrom && text === ',')) continue;
      let name = scan.nextSibling;
      while (name && (/Comment/.test(name.name) || /^(only|lateral)$/i.test(context.state.sliceDoc(name.from, name.to)))) name = name.nextSibling;
      if (!name || !/Identifier$/.test(name.name)) continue;
      const parts = path(context, name);
      const table = parts.length === 1 ? visible.get(parts[0]) : qualified.get(JSON.stringify(parts));
      if (!table) continue;
      const options = columns(table);
      fields.push(...options);
      local[namespaceKey(table.name)] = options;
      let alias = name.nextSibling;
      while (alias && /Comment/.test(alias.name)) alias = alias.nextSibling;
      if (alias && context.state.sliceDoc(alias.from, alias.to).toLowerCase() === 'as') alias = alias.nextSibling;
      if (alias && /^(Identifier|QuotedIdentifier)$/.test(alias.name)) local[namespaceKey(path(context, alias)[0])] = options;
    }
    const extra = schemaCompletionSource({ dialect: PostgreSQL, schema: local, tables: fields })(context);
    const primary = base(context);
    // These built-in completion sources are synchronous.
    if (primary instanceof Promise || extra instanceof Promise) return null;
    if (!primary) return extra;
    if (afterStatement) return { ...primary, options: primary.options.filter(option => option.type !== 'constant') };
    if (!extra || primary.from !== extra.from) return primary;
    const options = new Map<string, Completion>();
    for (const option of [...primary.options, ...extra.options]) options.set(`${option.type}:${option.label}`, option);
    return { ...primary, options: [...options.values()] };
  };
}
