import type { JobDefinition } from '~/lib/blueprints/registry';
import { cn } from '~/lib/utils';

interface FormSummaryProps {
  job: JobDefinition;
  values: Record<string, unknown>;
  context?: Record<string, unknown>;
}

export function FormSummary({ job, values, context }: FormSummaryProps) {
  return (
    <div className="glass-card rounded-lg p-4 space-y-2.5">
      {context &&
        Object.entries(context).map(([key, val]) => (
          <SummaryRow key={key} label={key} value={String(val)} mono />
        ))}
      {context && job.fields.length > 0 && (
        <div className="border-t border-cloud-elements-dividerColor my-2" />
      )}
      {job.fields
        .filter((f) => !f.internal)
        .map((field) => {
          const v = values[field.name];
          let display: string;
          if (field.type === 'boolean') {
            display = v ? 'Enabled' : 'Disabled';
          } else if (field.type === 'select' && field.options) {
            display = field.options.find((o) => o.value === String(v))?.label ?? String(v ?? '');
          } else {
            display = String(v ?? '--');
          }
          return <SummaryRow key={field.name} label={field.label} value={display} mono={field.type === 'json'} />;
        })}
    </div>
  );
}

function SummaryRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex justify-between text-sm">
      <span className="text-cloud-elements-textSecondary">{label}</span>
      <span className={cn('text-cloud-elements-textPrimary', mono ? 'font-data text-xs' : 'font-display')}>
        {value || '--'}
      </span>
    </div>
  );
}
