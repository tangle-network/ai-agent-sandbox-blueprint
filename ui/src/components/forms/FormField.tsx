import type { JobFieldDef } from '~/lib/blueprints/registry';
import { Input } from '~/components/ui/input';
import { Select } from '~/components/ui/select';
import { Textarea } from '~/components/ui/textarea';
import { Toggle } from '~/components/ui/toggle';
import { cn } from '~/lib/utils';

interface FormFieldProps {
  field: JobFieldDef;
  value: unknown;
  onChange: (name: string, value: unknown) => void;
  error?: string;
}

export function FormField({ field, value, onChange, error }: FormFieldProps) {
  if (field.internal) return null;

  const isBool = field.type === 'boolean';

  return (
    <div className={cn(isBool && 'flex items-center gap-3')}>
      {!isBool && (
        <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">
          {field.label}
          {field.required && ' *'}
        </label>
      )}
      <FieldInput field={field} value={value} onChange={onChange} />
      {isBool && (
        <span className="text-sm font-display text-cloud-elements-textSecondary">{field.label}</span>
      )}
      {field.helperText && !error && (
        <p className="text-xs text-cloud-elements-textTertiary mt-1">{field.helperText}</p>
      )}
      {error && <p className="text-xs text-crimson-400 mt-1">{error}</p>}
    </div>
  );
}

function FieldInput({
  field,
  value,
  onChange,
}: {
  field: JobFieldDef;
  value: unknown;
  onChange: (name: string, value: unknown) => void;
}) {
  switch (field.type) {
    case 'text':
      return (
        <Input
          value={String(value ?? '')}
          onChange={(e) => onChange(field.name, e.target.value)}
          placeholder={field.placeholder}
        />
      );
    case 'number':
      return (
        <Input
          type="number"
          value={value as number}
          onChange={(e) => onChange(field.name, Number(e.target.value))}
          placeholder={field.placeholder}
        />
      );
    case 'textarea':
    case 'json':
      return (
        <Textarea
          value={String(value ?? '')}
          onChange={(e) => onChange(field.name, e.target.value)}
          placeholder={field.placeholder}
          rows={field.type === 'json' ? 3 : 4}
          className={field.type === 'json' ? 'font-data text-sm' : undefined}
        />
      );
    case 'boolean':
      return <Toggle checked={Boolean(value)} onChange={(v) => onChange(field.name, v)} />;
    case 'select':
      return (
        <Select
          value={String(value ?? '')}
          onChange={(e) => onChange(field.name, e.target.value)}
          options={field.options ?? []}
        />
      );
    default:
      return null;
  }
}
