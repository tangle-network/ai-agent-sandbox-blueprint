import { useState, useCallback, useMemo } from 'react';
import type { JobDefinition } from '~/lib/blueprints/registry';

export interface JobFormState {
  values: Record<string, unknown>;
  errors: Record<string, string>;
  onChange: (name: string, value: unknown) => void;
  validate: () => boolean;
  reset: () => void;
}

function buildDefaults(job: JobDefinition): Record<string, unknown> {
  const init: Record<string, unknown> = {};
  for (const f of job.fields) {
    if (f.internal) continue;
    if (f.defaultValue !== undefined) {
      init[f.name] = f.defaultValue;
    } else if (f.type === 'boolean') {
      init[f.name] = false;
    } else if (f.type === 'number') {
      init[f.name] = 0;
    } else {
      init[f.name] = '';
    }
  }
  return init;
}

export function useJobForm(job: JobDefinition | null): JobFormState {
  const defaults = useMemo(() => (job ? buildDefaults(job) : {}), [job]);
  const [values, setValues] = useState<Record<string, unknown>>(defaults);
  const [errors, setErrors] = useState<Record<string, string>>({});

  const onChange = useCallback((name: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [name]: value }));
    setErrors((prev) => {
      if (!prev[name]) return prev;
      const next = { ...prev };
      delete next[name];
      return next;
    });
  }, []);

  const validate = useCallback((): boolean => {
    if (!job) return false;
    const errs: Record<string, string> = {};
    for (const f of job.fields) {
      if (!f.required || f.internal) continue;
      const v = values[f.name];
      if (v === undefined || v === null || v === '') {
        errs[f.name] = `${f.label} is required`;
      }
    }
    setErrors(errs);
    return Object.keys(errs).length === 0;
  }, [job, values]);

  const reset = useCallback(() => {
    setValues(defaults);
    setErrors({});
  }, [defaults]);

  return { values, errors, onChange, validate, reset };
}
