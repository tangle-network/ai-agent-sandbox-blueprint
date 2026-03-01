import { useState, useCallback, useEffect, useRef } from 'react';
import { Button } from '@tangle/blueprint-ui/components';
import { cn } from '@tangle/blueprint-ui';

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
        <div key={row.id} className="flex items-center gap-2">
          <input
            type="text"
            value={row.key}
            onChange={(e) => updateRow(row.id, 'key', e.target.value)}
            placeholder="KEY"
            className="flex-1 px-3 py-1.5 rounded-lg bg-cloud-elements-background-depth-2 border border-cloud-elements-borderColor text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus:outline-none focus:border-cloud-elements-borderColorActive transition-colors"
          />
          <span className="text-cloud-elements-textTertiary text-xs">=</span>
          <input
            type="text"
            value={row.value}
            onChange={(e) => updateRow(row.id, 'value', e.target.value)}
            placeholder="value"
            className="flex-[2] px-3 py-1.5 rounded-lg bg-cloud-elements-background-depth-2 border border-cloud-elements-borderColor text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus:outline-none focus:border-cloud-elements-borderColorActive transition-colors"
          />
          <button
            type="button"
            onClick={() => removeRow(row.id)}
            className="p-1.5 rounded-lg text-cloud-elements-textTertiary hover:text-crimson-400 hover:bg-crimson-500/10 transition-colors"
          >
            <div className="i-ph:x text-sm" />
          </button>
        </div>
      ))}
      <Button type="button" variant="ghost" size="sm" onClick={addRow} className="text-xs">
        <div className="i-ph:plus text-sm" />
        Add variable
      </Button>
    </div>
  );
}
