import { forwardRef, useEffect, useImperativeHandle, useRef } from 'react';
import { EditorState, Compartment } from '@codemirror/state';
import { EditorView, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection, keymap } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { sql, PostgreSQL, type SQLNamespace } from '@codemirror/lang-sql';
import { autocompletion, completionKeymap, closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
import { syntaxHighlighting, HighlightStyle, bracketMatching } from '@codemirror/language';
import { tags } from '@lezer/highlight';

export type EditorHandle = { selection: () => string; focus: () => void };
const syntax = HighlightStyle.define([{ tag: tags.keyword, color: 'var(--syntax-key)' }, { tag: [tags.string, tags.number], color: 'var(--syntax-value)' }, { tag: tags.comment, color: 'var(--muted)', fontStyle: 'italic' }, { tag: tags.operator, color: 'var(--muted)' }]);
const theme = EditorView.theme({ '&': { height: '100%', backgroundColor: 'transparent', color: 'var(--text)' }, '.cm-scroller': { fontFamily: 'var(--mono)', fontSize: '13px', lineHeight: '1.9' }, '.cm-content': { padding: '18px 0' }, '.cm-gutters': { background: 'transparent', border: 'none', color: 'var(--faint)' }, '.cm-lineNumbers .cm-gutterElement': { padding: '0 20px 0 18px' }, '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--text)' }, '.cm-activeLine': { background: 'var(--editor-line)' }, '&.cm-focused': { outline: 'none' }, '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { background: 'var(--selection) !important' }, '.cm-cursor': { borderLeftColor: 'var(--accent)' }, '.cm-tooltip': { background: 'var(--panel)', color: 'var(--text)', border: '1px solid var(--line)', borderRadius: '6px' } });
const SqlEditor = forwardRef<EditorHandle, { value: string; onChange: (value: string) => void; onRun: () => void; schema: SQLNamespace }>(({ value, onChange, onRun, schema }, ref) => {
  const host = useRef<HTMLDivElement>(null); const view = useRef<EditorView | null>(null); const callbacks = useRef({ onChange, onRun }); callbacks.current = { onChange, onRun };
  const language = useRef(new Compartment());
  useImperativeHandle(ref, () => ({ selection: () => { const v = view.current; return v ? v.state.sliceDoc(v.state.selection.main.from, v.state.selection.main.to) : ''; }, focus: () => view.current?.focus() }), []);
  useEffect(() => {
    if (!host.current) return;
    const instance = new EditorView({ parent: host.current, state: EditorState.create({ doc: value, extensions: [lineNumbers(), history(), highlightActiveLine(), highlightActiveLineGutter(), drawSelection(), bracketMatching(), closeBrackets(), autocompletion(), language.current.of(sql({ dialect: PostgreSQL, schema })), syntaxHighlighting(syntax), theme, EditorView.contentAttributes.of({ 'aria-label': 'SQL editor', spellcheck: 'false' }), keymap.of([{ key: 'Mod-Enter', run: () => { callbacks.current.onRun(); return true; } }, ...completionKeymap, ...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, indentWithTab]), EditorView.updateListener.of(update => { if (update.docChanged) callbacks.current.onChange(update.state.doc.toString()); })] }) });
    view.current = instance; return () => { view.current = null; instance.destroy(); };
    // The editor owns its document; external changes are synchronized below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => { const v = view.current; if (v && v.state.doc.toString() !== value) v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } }); }, [value]);
  useEffect(() => { view.current?.dispatch({ effects: language.current.reconfigure(sql({ dialect: PostgreSQL, schema })) }); }, [schema]);
  return <div className="sql-editor" ref={host} />;
});
export default SqlEditor;
