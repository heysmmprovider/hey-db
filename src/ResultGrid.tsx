import { memo, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { KeyRound, LockKeyhole } from 'lucide-react';
import type { CellEdit, QueryResult } from './types';
import { cellKey } from './data';

function ResultGrid({ result, edits, onEdit, disabled, onSelect }: { result: QueryResult; edits: CellEdit[]; onEdit: (edit: CellEdit) => void; disabled: boolean; onSelect: (cell: { row: number; column: number } | null) => void }) {
  const viewport = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ row: number; column: number; value: string } | null>(null);
  const changes = useMemo(() => new Map(edits.map(e => [cellKey(e.row, e.column), e.value])), [edits]);
  const widths = useMemo(() => result.columns.map((c, i) => Math.min(320, Math.max(c.dataType.includes('int') ? 125 : 180, c.name.length * 8 + 72, ...result.rows.slice(0, 15).map(r => Math.min((r[i]?.length ?? 4) * 7 + 36, 300))))), [result]);
  const vertical = useVirtualizer({ count: result.rows.length, getScrollElement: () => viewport.current, estimateSize: () => 34, overscan: 8, scrollMargin: 37 });
  const horizontal = useVirtualizer({ horizontal: true, count: result.columns.length, getScrollElement: () => viewport.current, estimateSize: i => widths[i], overscan: 2, paddingStart: 44 });
  const width = Math.max(horizontal.getTotalSize(), 500);
  const select = (row: number, column: number) => { setSelected(cellKey(row, column)); onSelect({ row, column }); };
  const begin = (row: number, column: number) => { select(row, column); if (!disabled && result.columns[column].editable) { const key = cellKey(row, column); setEditing({ row, column, value: (changes.has(key) ? changes.get(key) : result.rows[row][column]) ?? '' }); } };
  const commit = () => { if (editing) onEdit({ row: editing.row, column: editing.column, value: editing.value }); setEditing(null); };
  return <div className="grid-scroll" ref={viewport} role="grid" aria-label="Query results" aria-rowcount={result.rows.length + 1} aria-colcount={result.columns.length + 1}>
    <div className="grid-header" role="row" style={{ width, minWidth: '100%' }}>
      <div className="row-number header-number" role="columnheader">#</div>
      {horizontal.getVirtualItems().map(col => <div role="columnheader" className="grid-column" key={col.key} style={{ left: col.start, width: col.size }}><span>{result.columns[col.index].primaryKey && <KeyRound size={11} className="key-icon" />}{result.columns[col.index].name}</span><small>{result.columns[col.index].dataType}</small></div>)}
    </div>
    <div className="grid-body" style={{ height: vertical.getTotalSize(), width, minWidth: '100%' }}>
      {vertical.getVirtualItems().map(row => <div className="grid-row" role="row" aria-rowindex={row.index + 2} key={row.key} style={{ position: 'absolute', top: row.start - 37, height: row.size, width: '100%' }}>
        <div className="row-number" role="rowheader">{row.index + 1}</div>
        {horizontal.getVirtualItems().map(col => {
          const key = cellKey(row.index, col.index); const changed = changes.has(key); const value = changed ? changes.get(key)! : result.rows[row.index][col.index]; const isEditing = editing?.row === row.index && editing.column === col.index;
          return <div role="gridcell" aria-colindex={col.index + 2} aria-readonly={!result.columns[col.index].editable} aria-selected={selected === key} tabIndex={0} key={col.key} className={`grid-cell ${changed ? 'changed' : ''} ${selected === key ? 'selected' : ''} ${value === null ? 'null-value' : ''}`} style={{ left: col.start, width: col.size }} onClick={() => select(row.index, col.index)} onDoubleClick={() => begin(row.index, col.index)} onKeyDown={e => { if (!isEditing && (e.key === 'Enter' || e.key === 'F2')) { e.preventDefault(); begin(row.index, col.index); } }} title={value === null ? 'NULL' : value}>
            {isEditing ? <input autoCorrect="off" autoCapitalize="none" spellCheck={false} aria-label={`Edit ${result.columns[col.index].name}, row ${row.index + 1}`} autoFocus value={editing.value} onChange={e => setEditing({ ...editing, value: e.target.value })} onBlur={commit} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); commit(); } if (e.key === 'Escape') { e.preventDefault(); setEditing(null); } e.stopPropagation(); }} /> : <><span>{value === null ? 'NULL' : value === '' ? <span className="empty-value">empty string</span> : value}</span>{!result.columns[col.index].editable && result.columns[col.index].primaryKey && <LockKeyhole size={10} className="cell-lock" />}</>}
          </div>;
        })}
      </div>)}
    </div>
    {!result.rows.length && <div className="no-rows">No rows matched this query.</div>}
  </div>;
}

export default memo(ResultGrid);
