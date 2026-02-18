import type { JobDefinition } from '~/lib/blueprints/registry';
import { FormField } from './FormField';

export interface FormSection {
  label: string;
  fields: string[];
  collapsed?: boolean;
}

interface BlueprintJobFormProps {
  job: JobDefinition;
  values: Record<string, unknown>;
  onChange: (name: string, value: unknown) => void;
  errors?: Record<string, string>;
  sections?: FormSection[];
}

export function BlueprintJobForm({ job, values, onChange, errors, sections }: BlueprintJobFormProps) {
  const visibleFields = job.fields.filter((f) => !f.internal);

  if (sections) {
    return (
      <div className="space-y-6">
        {sections.map((section) => {
          const sectionFields = section.fields
            .map((name) => visibleFields.find((f) => f.name === name))
            .filter(Boolean);

          if (sectionFields.length === 0) return null;

          if (section.collapsed) {
            return (
              <details key={section.label} className="group">
                <summary className="cursor-pointer text-sm font-display font-medium text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary transition-colors">
                  {section.label}
                </summary>
                <div className="mt-4 space-y-4">
                  {sectionFields.map((field) => (
                    <FormField
                      key={field!.name}
                      field={field!}
                      value={values[field!.name]}
                      onChange={onChange}
                      error={errors?.[field!.name]}
                    />
                  ))}
                </div>
              </details>
            );
          }

          return (
            <div key={section.label}>
              <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-3">
                {section.label}
              </label>
              <div className="space-y-4">
                {sectionFields.map((field) => (
                  <FormField
                    key={field!.name}
                    field={field!}
                    value={values[field!.name]}
                    onChange={onChange}
                    error={errors?.[field!.name]}
                  />
                ))}
              </div>
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {visibleFields.map((field) => (
        <FormField
          key={field.name}
          field={field}
          value={values[field.name]}
          onChange={onChange}
          error={errors?.[field.name]}
        />
      ))}
    </div>
  );
}
