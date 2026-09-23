import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDownToLine, ArrowRight, Check, CheckCheck, ChevronDown, ChevronRight, CircleHelp, Code2, Database, FileCode2, KeyRound, LayoutPanelLeft, LoaderCircle, LockKeyhole, Moon, PanelLeftClose, Play, Plug, Plus, RefreshCw, Search, ShieldCheck, Square, Sun, Table2, Unplug, Undo2, X } from 'lucide-react';
import type { ReactNode } from 'react';
import type { SQLNamespace } from '@codemirror/lang-sql';
import { api, DEMO_ID, demoProfile, desktop } from './api';
import { DEFAULT_SQL, quoteIdentifier, stageEdit, toCsv } from './data';
import type { CellEdit, PlannedUpdate, Profile, QueryResult, TableDetails, TableInfo } from './types';
import SqlEditor from './SqlEditor';
import type { EditorHandle } from './SqlEditor';
import ResultGrid from './ResultGrid';

const message = (error: unknown) => error instanceof Error ? error.message : String(error);
type Tab = { id: string; name: string; sql: string };
function Dialog({ title, onClose, children, wide = false, busy = false }: { title: string; onClose: () => void; children: ReactNode; wide?: boolean; busy?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => { ref.current?.showModal(); }, []);
  return <dialog ref={ref} className={`dialog ${wide ? 'wide-dialog' : ''}`} onCancel={e => { e.preventDefault(); if (!busy) onClose(); }} aria-label={title}>
    <div className="dialog-heading"><h2>{title}</h2><button className="icon-button" aria-label="Close dialog" disabled={busy} onClick={onClose}><X size={18} /></button></div>{children}
  </dialog>;
}

function ConnectionDialog({ existing, onClose, onConnect }: { existing?: Profile; onClose: () => void; onConnect: (profile: Profile, password: string | undefined) => Promise<void> }) {
  const [profile, setProfile] = useState<Profile>(existing ?? { id: crypto.randomUUID(), name: '', host: 'localhost', port: 5432, database: 'postgres', username: 'postgres', tls: 'verify-full', readOnly: false, rememberPassword: false });
  const [password, setPassword] = useState(''); const [touched, setTouched] = useState(false); const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const field = (key: keyof Profile, value: string | number | boolean) => setProfile(p => ({ ...p, [key]: value }));
  return <Dialog title={existing ? 'Connect to PostgreSQL' : 'New connection'} onClose={onClose} busy={busy}>
    <div className="connection-intro"><span className="database-badge"><Database size={23} /></span><div><strong>PostgreSQL</strong><p>A direct connection from your computer.</p></div></div>
    <form onSubmit={async e => { e.preventDefault(); setBusy(true); setError(''); try { await onConnect({ ...profile, name: profile.name.trim() || profile.database }, existing?.rememberPassword && !touched ? undefined : password); onClose(); } catch (err) { setError(message(err)); } finally { setBusy(false); } }}>
      <fieldset disabled={busy}>
        <label>Connection name<input autoFocus placeholder="e.g. Local development" value={profile.name} onChange={e => field('name', e.target.value)} /></label>
        <div className="form-row host-row"><label>Host<input required value={profile.host} onChange={e => field('host', e.target.value)} spellCheck={false} /></label><label>Port<input required type="number" min="1" max="65535" value={profile.port} onChange={e => field('port', Number(e.target.value))} /></label></div>
        <label>Database<input required value={profile.database} onChange={e => field('database', e.target.value)} spellCheck={false} /></label>
        <div className="form-row"><label>Username<input required value={profile.username} onChange={e => field('username', e.target.value)} autoComplete="off" spellCheck={false} /></label><label>Password<input type="password" value={password} placeholder={existing?.rememberPassword ? 'Use saved password' : 'Password'} onChange={e => { setTouched(true); setPassword(e.target.value); }} autoComplete="off" /></label></div>
        <label>Transport security<select value={profile.tls} onChange={e => field('tls', e.target.value)}><option value="verify-full">TLS · Verify certificate and hostname</option><option value="disable">No TLS · Local development only</option></select></label>
        <label className="check-label"><input type="checkbox" checked={profile.rememberPassword} onChange={e => field('rememberPassword', e.target.checked)} />Remember password in system credential storage</label>
        <label className="check-label"><input type="checkbox" checked={profile.readOnly} onChange={e => field('readOnly', e.target.checked)} />Read-only connection</label>
      </fieldset>
      {error && <div className="form-error" role="alert">{error}</div>}
      {!desktop && <div className="form-error">Real connections are available in the desktop app. Use the demo in this browser preview.</div>}
      <div className="dialog-footer"><span><ShieldCheck size={13} /> Stored only on this computer</span><button className="primary-button" disabled={busy || !desktop} type="submit">{busy ? <LoaderCircle size={15} className="spin" /> : <Plug size={15} />}{busy ? 'Connecting…' : 'Connect & save'}</button></div>
    </form>
  </Dialog>;
}

export default function App() {
  const [profiles, setProfiles] = useState<Profile[]>([]); const [active, setActive] = useState<Profile | null>(null);
  const [tables, setTables] = useState<TableInfo[]>([]); const [filter, setFilter] = useState(''); const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [tabs, setTabs] = useState<Tab[]>([{ id: 'first', name: 'Query 1', sql: DEFAULT_SQL }]); const [tabId, setTabId] = useState('first');
  const [result, setResult] = useState<QueryResult | null>(null); const [details, setDetails] = useState<TableDetails | null>(null); const [selectedTable, setSelectedTable] = useState<TableInfo | null>(null);
  const [edits, setEdits] = useState<CellEdit[]>([]); const [cell, setCell] = useState<{ row: number; column: number } | null>(null);
  const [busy, setBusy] = useState(false); const [applying, setApplying] = useState(false); const [operation, setOperation] = useState<string | null>(null);
  const [error, setError] = useState(''); const [notice, setNotice] = useState(''); const [view, setView] = useState<'results' | 'structure'>('results');
  const [connectionDialog, setConnectionDialog] = useState<{ existing?: Profile } | null>(null); const [connectionMenu, setConnectionMenu] = useState(false);
  const [review, setReview] = useState<PlannedUpdate[] | null>(null); const [reviewError, setReviewError] = useState(''); const [discard, setDiscard] = useState<(() => void) | null>(null);
  const [sidebar, setSidebar] = useState(true); const [help, setHelp] = useState(false);
  const [appearance, setAppearance] = useState(() => localStorage.getItem('heydb.appearance') || 'system');
  const [editorHeight, setEditorHeight] = useState(270);
  const editor = useRef<EditorHandle>(null); const lastSql = useRef(''); const inFlight = useRef(false);
  const currentTab = tabs.find(t => t.id === tabId)!;
  useEffect(() => { api.profiles().then(setProfiles).catch(err => setError(message(err))); }, []);
  useEffect(() => { document.documentElement.dataset.theme = appearance; document.documentElement.style.colorScheme = appearance === 'system' ? 'light dark' : appearance; localStorage.setItem('heydb.appearance', appearance); }, [appearance]);
  useEffect(() => { const fn = (e: BeforeUnloadEvent) => { if (edits.length) { e.preventDefault(); e.returnValue = ''; } }; window.addEventListener('beforeunload', fn); return () => window.removeEventListener('beforeunload', fn); }, [edits.length]);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false; let unsubscribe: (() => void) | undefined;
    void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
      const window = getCurrentWindow();
      const stop = await window.onCloseRequested(event => {
        if (busy || applying) { event.preventDefault(); setError('Finish or cancel the current operation before closing the window.'); }
        else if (edits.length) { event.preventDefault(); setDiscard(() => () => { void window.destroy(); }); }
      });
      if (disposed) stop(); else unsubscribe = stop;
    });
    return () => { disposed = true; unsubscribe?.(); };
  }, [edits.length, busy, applying]);
  const guard = (action: () => void) => { if (busy || applying) return; if (edits.length) setDiscard(() => action); else action(); };
  const resetResult = () => { setResult(null); setEdits([]); setCell(null); setDetails(null); setSelectedTable(null); setError(''); setNotice(''); setView('results'); };
  const loadTables = async (profile: Profile) => { try { setTables(await api.tables(profile.id)); } catch (err) { setError(message(err)); } };
  const run = useCallback(async (sqlOverride?: string, profileOverride?: Profile, ignoreEdits = false) => {
    const profile = profileOverride ?? active; if (!profile || inFlight.current) return;
    if (edits.length && !ignoreEdits) { setError('Apply or discard your pending changes before running another query.'); return; }
    const sql = sqlOverride ?? (editor.current?.selection().trim() || currentTab.sql);
    if (!sql.trim()) return;
    const operationId = crypto.randomUUID(); inFlight.current = true; setBusy(true); setOperation(operationId); setError(''); setNotice(''); setCell(null);
    try { const data = await api.query(profile.id, sql, operationId); lastSql.current = sql; setResult(data); setEdits([]); setView('results'); setNotice(data.columns.length ? '' : `Statement completed · ${data.affectedRows} row${data.affectedRows === 1 ? '' : 's'} affected`); }
    catch (err) { setError(message(err)); setResult(null); }
    finally { inFlight.current = false; setBusy(false); setOperation(null); }
  }, [active, currentTab.sql, edits.length]);
  const connect = async (profile: Profile, password?: string) => {
    if (inFlight.current) throw new Error('Wait for the running query.');
    await api.connect(profile, password);
    try { await api.saveProfile(profile, password); } catch (err) { await api.disconnect(profile.id); throw err; }
    if (active && active.id !== profile.id) await api.disconnect(active.id);
    setProfiles(await api.profiles()); setActive(profile); resetResult(); setConnectionMenu(false); await loadTables(profile); setNotice(`Connected to ${profile.name}`);
  };
  const enterDemo = async () => { if (active) await api.disconnect(active.id); setActive(demoProfile); resetResult(); setTabs([{ id: 'demo-query', name: 'Active products.sql', sql: DEFAULT_SQL }]); setTabId('demo-query'); await loadTables(demoProfile); await run(DEFAULT_SQL, demoProfile, true); };
  const browse = (table: TableInfo) => guard(() => {
    if (!active) return;
    const query = `SELECT *\nFROM ${quoteIdentifier(table.schema)}.${quoteIdentifier(table.name)}\nLIMIT 1000;`;
    setTabs(previous => previous.map(t => t.id === tabId ? { ...t, name: table.name, sql: query } : t)); setSelectedTable(table); setDetails(null);
    api.details(active.id, table.oid).then(setDetails).catch(err => setError(message(err))).then(() => run(query, active, true));
  });
  const stage = (edit: CellEdit) => { if (result && !applying) { setEdits(current => stageEdit(current, result, edit)); setNotice(''); } };
  const preview = async () => {
    if (!active || !result || !edits.length || busy || applying) return;
    try { setReview(await api.preview(active.id, result.id, edits)); setReviewError(''); } catch (err) { setError(message(err)); }
  };
  const apply = async () => {
    if (!active || !result || applying) return; setApplying(true); setReviewError('');
    let committed = false;
    try { const count = await api.apply(active.id, result.id, edits); committed = true; setEdits([]); setReview(null); await run(lastSql.current, active, true); setNotice(`${active.id === DEMO_ID ? 'Demo: ' : ''}${count} row${count === 1 ? '' : 's'} updated successfully.`); }
    catch (err) { if (committed) { setReview(null); setResult(null); setError(`Changes applied, but refresh failed: ${message(err)}`); } else setReviewError(message(err)); }
    finally { setApplying(false); }
  };
  const exportCsv = async () => {
    if (!result) return;
    try { const csv = toCsv(result); if (desktop) { const { save } = await import('@tauri-apps/plugin-dialog'); const { writeTextFile } = await import('@tauri-apps/plugin-fs'); const path = await save({ defaultPath: 'query-results.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] }); if (path) { await writeTextFile(path, csv); setNotice('Saved loaded results to CSV. Pending edits are excluded.'); } } else { const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' })); const link = document.createElement('a'); link.href = url; link.download = 'query-results.csv'; link.click(); URL.revokeObjectURL(url); } } catch (err) { setError(message(err)); }
  };
  const newTab = () => guard(() => { const id = crypto.randomUUID(); setTabs(previous => [...previous, { id, name: `Query ${previous.length + 1}`, sql: '' }]); setTabId(id); resetResult(); });
  const schemas = useMemo(() => [...new Set(tables.map(t => t.schema))], [tables]);
  const completions = useMemo(() => { const schema: Record<string, Record<string, string[]>> = {}; for (const table of tables) { schema[table.schema] ??= {}; schema[table.schema][table.name] = selectedTable?.oid === table.oid && details ? details.columns.map(c => c.name) : []; } return schema as SQLNamespace; }, [tables, selectedTable, details]);
  const pendingRows = new Set(edits.map(e => e.row)).size;
  const switchTheme = () => setAppearance(value => value === 'system' ? 'dark' : value === 'dark' ? 'light' : 'system');
  useEffect(() => { const fn = (event: KeyboardEvent) => { if (event.target instanceof HTMLElement && event.target.closest('dialog')) return; if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') { event.preventDefault(); void preview(); } }; window.addEventListener('keydown', fn); return () => window.removeEventListener('keydown', fn); });

  return <div className={`app ${sidebar ? '' : 'sidebar-hidden'} ${!desktop ? 'browser-preview' : ''}`}>
    <header className="titlebar" data-tauri-drag-region>
      {!desktop && <div className="traffic-lights" aria-hidden="true"><i /><i /><i /></div>}
      <span className="wordmark" data-tauri-drag-region>hey db<span className="wordmark-dot">.</span></span>
      <div className="window-title" data-tauri-drag-region>{active ? active.name : 'PostgreSQL workspace'}{active?.id === DEMO_ID && <span className="demo-label">DEMO</span>}</div>
      <div className="title-actions"><button className="icon-button" title="Toggle sidebar" aria-label="Toggle sidebar" onClick={() => setSidebar(!sidebar)}><LayoutPanelLeft size={16} /></button><button className="icon-button" title={`Appearance: ${appearance}. Click to cycle.`} aria-label={`Appearance: ${appearance}`} onClick={switchTheme}>{appearance === 'dark' ? <Moon size={16} /> : <Sun size={16} />}</button><button className="icon-button" title="Help and shortcuts" aria-label="Help and shortcuts" onClick={() => setHelp(true)}><CircleHelp size={16} /></button></div>
    </header>
    <div className="workspace">
      {sidebar && <aside className="sidebar">
        <div className="sidebar-heading"><span>CONNECTIONS</span><div><button className="icon-button" aria-label="New connection" title="New connection" disabled={busy || applying} onClick={() => guard(() => setConnectionDialog({}))}><Plus size={16} /></button><button className="icon-button" aria-label="Collapse sidebar" title="Collapse sidebar" onClick={() => setSidebar(false)}><PanelLeftClose size={15} /></button></div></div>
        <div className="connection-picker"><button className={`connection-card ${active ? 'is-connected' : ''}`} onClick={() => setConnectionMenu(!connectionMenu)} aria-expanded={connectionMenu}><span className="connection-icon"><Database size={19} /></span><span className="connection-label"><strong>{active?.name ?? 'Choose a connection'}</strong><small>{active ? <><i className="status-dot" />{active.id === DEMO_ID ? 'Local demo' : `${active.host}:${active.port}`}</> : 'PostgreSQL'}</small></span><ChevronDown size={14} /></button>
          {connectionMenu && <div className="connection-menu">{profiles.map(profile => <button key={profile.id} onClick={() => guard(() => { setConnectionDialog({ existing: profile }); setConnectionMenu(false); })}><Database size={14} /><span>{profile.name}</span>{profile.id === active?.id && <Check size={14} />}</button>)}<button onClick={() => guard(() => { setConnectionDialog({}); setConnectionMenu(false); })}><Plus size={14} />New connection</button><button onClick={() => guard(() => { setConnectionMenu(false); void enterDemo(); })}><Code2 size={14} />Explore demo</button>{active && <button onClick={() => guard(() => { void api.disconnect(active.id).then(() => { setActive(null); setTables([]); resetResult(); setConnectionMenu(false); }); })}><Unplug size={14} />Disconnect</button>}</div>}
        </div>
        {active && <><div className="sidebar-search"><Search size={14} /><input aria-label="Filter tables" placeholder="Find a table…" value={filter} onChange={e => setFilter(e.target.value)} /><kbd>⌕</kbd></div><div className="explorer-heading"><span><ChevronDown size={13} /><Database size={13} />{active.database}</span><button className="icon-button" disabled={busy || applying} aria-label="Refresh schema" title="Refresh schema" onClick={() => void loadTables(active)}><RefreshCw size={12} /></button></div>
          <nav className="schema-tree" aria-label="Database schema">{schemas.map(schema => <div key={schema}><button className="schema-heading" onClick={() => setCollapsed(current => { const next = new Set(current); next.has(schema) ? next.delete(schema) : next.add(schema); return next; })}>{collapsed.has(schema) ? <ChevronRight size={13} /> : <ChevronDown size={13} />}<span>{schema}</span><small>{tables.filter(t => t.schema === schema).length}</small></button>{!collapsed.has(schema) && tables.filter(t => t.schema === schema && `${t.schema}.${t.name}`.toLowerCase().includes(filter.toLowerCase())).map(table => <button className={`table-item ${selectedTable?.oid === table.oid || (!selectedTable && active.id === DEMO_ID) ? 'active' : ''}`} key={table.oid} disabled={busy || applying} onClick={() => browse(table)} title={`${table.schema}.${table.name}`}><Table2 size={14} /><span>{table.name}</span>{['v', 'm'].includes(table.kind) && <small>view</small>}</button>)}</div>)}{!tables.length && <p className="sidebar-empty">No user tables found.</p>}</nav></>}
        {!active && <div className="sidebar-empty">Your saved databases<br />will appear here.</div>}
        <div className="sidebar-bottom"><span className="pg-logo"><Database size={13} /></span><span>PostgreSQL</span><span className="version">v0.1.0</span></div>
      </aside>}
      <main className="main">
        <div className="tabbar"><div className="tabs">{tabs.map(tab => <button key={tab.id} className={`query-tab ${tab.id === tabId ? 'active' : ''}`} onClick={() => { if (tab.id !== tabId) guard(() => { setTabId(tab.id); resetResult(); }); }}><FileCode2 size={14} /><span>{tab.name}</span>{tab.id === tabId && edits.length > 0 && <i className="pending-dot" />}</button>)}<button className="icon-button new-tab" onClick={newTab} aria-label="New query" title="New query"><Plus size={16} /></button></div><span className="tab-language">SQL</span></div>
        {!active ? <div className="welcome"><div className="welcome-mark"><Database size={33} strokeWidth={1.5} /></div><div className="eyebrow">A FOCUSED DATABASE WORKSPACE</div><h1>Make yourself at home<br />in your database.</h1><p>Explore your tables, write SQL, and make thoughtful changes.<br />Everything stays on your computer.</p><div className="welcome-actions"><button className="primary-button" onClick={() => setConnectionDialog({})}><Plus size={16} />Connect to PostgreSQL</button><button className="secondary-button" onClick={() => void enterDemo()}>Explore the demo<ArrowRight size={15} /></button></div><div className="welcome-footnote"><ShieldCheck size={14} />No account. No telemetry. Just your workspace.</div></div> : <>
          <div className="query-toolbar"><div className="query-context"><i className="status-dot" /><strong>{active.database}</strong><ChevronRight size={12} /><span>{selectedTable?.schema ?? 'public'}</span>{active.readOnly && <span className="readonly-badge"><LockKeyhole size={11} />Read-only</span>}</div><div className="query-actions"><span className="selection-hint">Selected SQL or whole editor</span>{busy ? <button className="stop-button" disabled={applying || active.id === DEMO_ID} onClick={() => { if (operation) void api.cancel(active.id, operation).catch(err => setError(message(err))); }}><Square size={12} fill="currentColor" />Cancel</button> : <button className="run-button" disabled={applying || edits.length > 0} onClick={() => void run()}><Play size={13} fill="currentColor" />Run<kbd>⌘ ↵</kbd></button>}</div></div>
          <section className="editor-pane" style={{ height: editorHeight }} aria-label="Query editor"><SqlEditor ref={editor} value={currentTab.sql} schema={completions} onChange={sql => setTabs(current => current.map(t => t.id === tabId ? { ...t, sql } : t))} onRun={() => void run()} /></section>
          <div className="pane-divider" role="separator" aria-label="Resize SQL editor" aria-orientation="horizontal" tabIndex={0} onKeyDown={e => { if (e.key === 'ArrowUp' || e.key === 'ArrowDown') { e.preventDefault(); setEditorHeight(h => Math.max(110, Math.min(window.innerHeight - 340, h + (e.key === 'ArrowUp' ? -20 : 20)))); } }} onPointerDown={e => { e.currentTarget.setPointerCapture(e.pointerId); }} onPointerMove={e => { if (e.currentTarget.hasPointerCapture(e.pointerId)) setEditorHeight(h => Math.max(110, Math.min(window.innerHeight - 340, h + e.movementY))); }} onPointerUp={e => e.currentTarget.releasePointerCapture(e.pointerId)}><span /></div>
          <section className="results-pane" aria-label="Results workspace"><div className="results-toolbar"><div className="result-tabs"><button className={view === 'results' ? 'active' : ''} onClick={() => setView('results')}><Table2 size={14} />Results{result && <span className="count">{result.rows.length.toLocaleString()}</span>}</button><button className={view === 'structure' ? 'active' : ''} disabled={!details} onClick={() => setView('structure')}>Structure</button></div><div className="result-actions">{cell && result?.columns[cell.column].editable && <button className="text-button" disabled={busy || applying} onClick={() => stage({ ...cell, value: null })}>Set NULL</button>}<button className="icon-button" aria-label="Refresh results" title="Refresh results" disabled={!result || busy || applying || edits.length > 0} onClick={() => void run(lastSql.current)}><RefreshCw size={14} /></button><button className="text-button" disabled={!result || !result.columns.length || busy || applying} onClick={() => void exportCsv()}><ArrowDownToLine size={14} />Export</button></div></div>
          {error && <div className="notice error" role="alert"><span>{error}</span><button className="icon-button" onClick={() => setError('')} aria-label="Dismiss error"><X size={14} /></button></div>}
          {notice && <div className="notice success" role="status"><CheckCheck size={15} /><span>{notice}</span><button className="icon-button" onClick={() => setNotice('')} aria-label="Dismiss notification"><X size={14} /></button></div>}
          {result?.truncated && <div className="notice warning">Showing up to 1,000 rows / 8 MiB. Remaining results were canceled. Use LIMIT and filters to narrow your query.</div>}
          {edits.length > 0 && <div className="pending-bar"><span><i className="pending-dot" /><strong>{edits.length} pending {edits.length === 1 ? 'change' : 'changes'}</strong><small>across {pendingRows} {pendingRows === 1 ? 'row' : 'rows'}</small></span><div><button className="text-button" disabled={applying} onClick={() => setDiscard(() => () => { setEdits([]); setNotice('Pending changes discarded.'); })}><Undo2 size={14} />Discard</button><button className="apply-button" disabled={applying || busy} onClick={() => void preview()}><Play size={12} fill="currentColor" />Apply changes<kbd>⌘ S</kbd></button></div></div>}
          {view === 'structure' && details ? <div className="structure-view"><table><thead><tr><th>Column</th><th>Type</th><th>Nullable</th><th>Default</th></tr></thead><tbody>{details.columns.map(column => <tr key={column.name}><td>{column.primaryKey && <KeyRound size={12} className="key-icon" />}{column.name}</td><td><code>{column.dataType}</code></td><td>{column.nullable ? 'Yes' : 'No'}</td><td><code>{column.defaultValue ?? '—'}</code></td></tr>)}</tbody></table><h3>Indexes</h3>{details.indexes.map(index => <div className="index-definition" key={index.name}><strong>{index.name}</strong><code>{index.definition}</code></div>)}</div> : result?.columns.length ? <ResultGrid key={result.id} result={result} edits={edits} onEdit={stage} onSelect={setCell} disabled={busy || applying} /> : <div className="result-empty">{busy ? <LoaderCircle size={23} className="spin" /> : <Table2 size={25} strokeWidth={1.4} />}<strong>{busy ? 'Running your query…' : result ? 'Statement complete' : 'A place for your results'}</strong><p>{busy ? 'You can cancel a running query at any time.' : result ? `${result.affectedRows} rows affected.` : 'Run a query or choose a table in the sidebar.'}</p>{!busy && !result && <span className="shortcut"><kbd>⌘</kbd><kbd>↵</kbd> to run SQL</span>}</div>}
          {result && <div className="result-footer"><span>{result.readOnlyReason ? <><LockKeyhole size={11} />{result.readOnlyReason}</> : <><span className="editable-dot" />Double-click a cell to edit · Enter to save · Esc to cancel</>}</span><span>{active.id === DEMO_ID ? 'Sample data' : `${result.elapsedMs.toLocaleString()} ms`}</span></div>}</section>
        </>}
      </main>
    </div>
    <footer className="statusbar"><span><i className={`status-dot ${!active ? 'offline' : ''}`} />{active ? active.id === DEMO_ID ? 'Demo workspace · No database connected' : `${active.name} · Connected` : 'Ready when you are'}{active?.readOnly && <LockKeyhole size={11} />}</span><span>{active && <><span>PostgreSQL</span><i className="status-divider" /></>}UTF-8<span className="status-divider" />Local first</span></footer>
    {connectionDialog && <ConnectionDialog existing={connectionDialog.existing} onClose={() => setConnectionDialog(null)} onConnect={connect} />}
    {discard && <Dialog title="Discard pending changes?" onClose={() => setDiscard(null)}><p className="dialog-copy">You have {edits.length} unapplied cell {edits.length === 1 ? 'edit' : 'edits'}. Discarding restores the loaded values.</p><div className="dialog-footer"><button className="secondary-button" onClick={() => setDiscard(null)}>Keep editing</button><button className="danger-button" onClick={() => { const next = discard; setEdits([]); setDiscard(null); next(); }}>Discard changes</button></div></Dialog>}
    {review && <Dialog title="Review your changes" wide busy={applying} onClose={() => setReview(null)}><div className="review-summary"><span className="review-icon"><CheckCheck size={22} /></span><div><strong>{review.length} {review.length === 1 ? 'row' : 'rows'} will be updated</strong><p>{active?.id === DEMO_ID ? 'Demo only. Changes affect the sample data in this session.' : 'All updates apply together. Conflicts or errors roll back the entire batch.'}</p></div></div><div className="review-list">{review.map((update, index) => <section className="update-preview" key={update.row}><div className="update-title">UPDATE {index + 1}<span>Result row {update.row + 1}</span></div><pre>{update.sql}</pre><div className="parameter-list">{update.parameters.map((value, i) => <div key={i}><code>${i + 1}</code><span className={value === null ? 'null-value' : ''}>{value === null ? 'NULL' : JSON.stringify(value)}</span></div>)}</div></section>)}</div>{reviewError && <div className="form-error" role="alert">{reviewError}</div>}<div className="dialog-footer"><button className="secondary-button" disabled={applying} onClick={() => setReview(null)}>Back to editing</button><button className="primary-button" disabled={applying} onClick={() => void apply()}>{applying ? <LoaderCircle size={14} className="spin" /> : <Play size={13} fill="currentColor" />}{applying ? 'Applying…' : `Apply ${review.length} ${review.length === 1 ? 'update' : 'updates'}`}</button></div></Dialog>}
    {help && <Dialog title="A few useful things" onClose={() => setHelp(false)}><div className="help-list"><p><kbd>⌘ / Ctrl + Enter</kbd><span>Run selected SQL or the whole editor</span></p><p><kbd>Double-click / Enter</kbd><span>Edit a selected result cell</span></p><p><kbd>⌘ / Ctrl + S</kbd><span>Review and apply pending edits</span></p><p><kbd>Esc</kbd><span>Cancel a cell edit or close a dialog</span></p><hr /><p>Queries run one statement at a time. Results are limited to 1,000 rows or 8 MiB. Include the table’s full primary key to edit direct, single-table results. Keys and generated columns stay read-only.</p><p>SQL and results stay in memory. Saved connections live in your application settings; remembered passwords use your system credential store. CSV exports contain the loaded rows, without pending edits.</p><p>v0.1.0 · PostgreSQL · Open source</p></div></Dialog>}
    {!active && error && <div className="global-error" role="alert">{error}<button className="icon-button" aria-label="Dismiss error" onClick={() => setError('')}><X size={14} /></button></div>}
  </div>;
}
