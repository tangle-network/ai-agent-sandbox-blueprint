import { useState, useCallback, useEffect, useRef } from 'react';
import { cn } from '@tangle-network/blueprint-ui';

interface EnvEditorProps {
  value: string;
  onChange: (json: string) => void;
  className?: string;
}

interface EnvRow {
  id: number;
  key: string;
  value: string;
}

let nextId = 1;

const envInputClass = 'min-h-11 min-w-0 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3.5 py-2.5 font-data text-[15px] font-medium text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow)] placeholder:text-[var(--sandbox-console-subtle)] transition-[background-color,border-color,box-shadow,color] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] focus:border-[var(--sandbox-console-brand-border)] focus:bg-[var(--sandbox-console-control-hover)] focus:shadow-[var(--sandbox-console-control-shadow-focus)] focus:outline-none';

function parseJsonToRows(json: string): EnvRow[] {
  try {
    const obj = JSON.parse(json);
    if (typeof obj !== 'object' || obj === null || Array.isArray(obj)) return [];
    const entries = Object.entries(obj);
    if (entries.length === 0) return [];
    return entries.map(([key, val]) => ({ id: nextId++, key, value: String(val) }));
  } catch {
    return [];
  }
}

function rowsToJson(rows: EnvRow[]): string {
  const obj: Record<string, string> = {};
  for (const row of rows) {
    const k = row.key.trim();
    if (k) obj[k] = row.value;
  }
  return JSON.stringify(obj);
}

export function EnvEditor({ value, onChange, className }: EnvEditorProps) {
  const [rows, setRows] = useState<EnvRow[]>(() => parseJsonToRows(value));
  const suppressSync = useRef(false);

  // Sync external value changes (e.g. form reset)
  useEffect(() => {
    if (suppressSync.current) {
      suppressSync.current = false;
      return;
    }
    const parsed = parseJsonToRows(value);
    if (parsed.length > 0 || value === '{}' || !value) {
      setRows(parsed);
    }
  }, [value]);

  const emitChange = useCallback(
    (updated: EnvRow[]) => {
      setRows(updated);
      suppressSync.current = true;
      onChange(rowsToJson(updated));
    },
    [onChange],
  );

  const addRow = useCallback(() => {
    emitChange([...rows, { id: nextId++, key: '', value: '' }]);
  }, [rows, emitChange]);

  const removeRow = useCallback(
    (id: number) => {
      emitChange(rows.filter((r) => r.id !== id));
    },
    [rows, emitChange],
  );

  const updateRow = useCallback(
    (id: number, field: 'key' | 'value', val: string) => {
      emitChange(rows.map((r) => (r.id === id ? { ...r, [field]: val } : r)));
    },
    [rows, emitChange],
  );

  return (
    <div className={cn('space-y-2', className)}>
      {rows.map((row) => (
        <div key={row.id} className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1.35fr)_auto] items-center gap-2">
          <input
            type="text"
            value={row.key}
            onChange={(e) => updateRow(row.id, 'key', e.target.value)}
            placeholder="KEY"
            className={envInputClass}
          />
          <span className="font-data text-sm font-bold text-[var(--sandbox-console-subtle)]">=</span>
          <input
            type="text"
            value={row.value}
            onChange={(e) => updateRow(row.id, 'value', e.target.value)}
            placeholder="value"
            className={envInputClass}
          />
          <button
            type="button"
            onClick={() => removeRow(row.id)}
            className="flex h-11 w-11 items-center justify-center rounded-[5px] text-[var(--sandbox-console-muted)] transition-colors hover:bg-red-400/10 hover:text-[var(--sandbox-console-danger)]"
            aria-label="Remove environment variable"
          >
            <div className="i-ph:x text-sm" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={addRow}
        className="inline-flex h-9 items-center gap-2 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3 font-display text-sm font-bold text-[var(--sandbox-console-secondary)] shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] hover:text-[var(--sandbox-console-text)]"
      >
        <div className="i-ph:plus text-sm" />
        Add variable
      </button>
    </div>
  );
}
