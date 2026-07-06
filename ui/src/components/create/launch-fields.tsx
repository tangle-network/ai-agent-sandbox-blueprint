import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type RefObject,
  type TextareaHTMLAttributes,
} from 'react';
import { cn, type JobDefinition, type JobFieldDef } from '@tangle-network/blueprint-ui';
import {
  IdentityMark,
  getAgentIdentity,
  getCapabilityIdentity,
  getImageIdentity,
  getResourceIdentity,
  getRuntimeIdentity,
  type IdentityMeta,
} from '~/components/shared/VisualIdentity';
import {
  BUNDLED_AGENT_OPTIONS,
  BUNDLED_NO_AGENT_VALUE,
  sanitizeBundledAgentIdentifier,
} from '~/lib/agents';
import {
  CUSTOM_IMAGE_VALUE,
  clampNumber,
  field,
  formatImageOptionLabel,
  valueNumber,
  type LaunchSelectOption,
} from './support';

export function LaunchActionButton({
  variant = 'primary',
  size = 'md',
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'primary' | 'secondary' | 'danger' | 'success';
  size?: 'sm' | 'md' | 'lg';
}) {
  return (
    <button
      type="button"
      className={cn(
        'inline-flex items-center justify-center gap-2 rounded-[5px] border font-display font-bold transition-[background-color,border-color,box-shadow,color,transform] duration-150 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50',
        size === 'sm' && 'h-9 px-3 text-sm',
        size === 'md' && 'h-11 px-4 text-[15px]',
        size === 'lg' && 'h-12 px-5 text-[15px]',
        variant === 'primary' && 'border-[var(--sandbox-console-brand-border)] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--sandbox-console-brand)_22%,var(--sandbox-console-panel-strong)),var(--sandbox-console-brand-soft))] text-[var(--sandbox-console-text)] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] hover:border-[var(--sandbox-console-brand)] hover:bg-[rgba(142,89,255,0.26)] hover:shadow-[0_0_0_3px_rgba(168,123,255,0.13),inset_0_1px_0_rgba(255,255,255,0.08)]',
        variant === 'secondary' && 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)] shadow-[var(--sandbox-console-control-shadow)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] hover:text-[var(--sandbox-console-text)]',
        variant === 'success' && 'border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)] hover:bg-[rgba(56,178,172,0.20)]',
        variant === 'danger' && 'border-red-400/20 bg-red-400/10 text-[var(--sandbox-console-danger)] hover:bg-red-400/15',
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}

export function LaunchField({
  label,
  detail,
  error,
  children,
}: {
  label: string;
  detail?: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <label className="block min-w-0 space-y-2">
      <span className="flex items-center justify-between gap-3">
        <span className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">{label}</span>
        {detail ? <span className="font-data text-xs font-medium text-[var(--sandbox-console-subtle)]">{detail}</span> : null}
      </span>
      {children}
      {error ? <span className="block text-sm text-[var(--sandbox-console-danger)]">{error}</span> : null}
    </label>
  );
}

export const launchControlClass = 'min-h-11 w-full rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3.5 py-2.5 font-data text-[15px] font-medium text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow)] placeholder:text-[var(--sandbox-console-subtle)] transition-[background-color,border-color,box-shadow,color] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] focus:border-[var(--sandbox-console-brand-border)] focus:bg-[var(--sandbox-console-control-hover)] focus:shadow-[var(--sandbox-console-control-shadow-focus)] focus:outline-none disabled:cursor-not-allowed disabled:opacity-60';

export function LaunchInput({
  label,
  detail,
  error,
  inputRef,
  className,
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, 'ref'> & {
  label: string;
  detail?: string;
  error?: string;
  inputRef?: RefObject<HTMLInputElement>;
}) {
  return (
    <LaunchField label={label} detail={detail} error={error}>
      <input ref={inputRef} aria-label={label} className={cn(launchControlClass, className)} {...props} />
    </LaunchField>
  );
}

export function LaunchTextArea({
  label,
  detail,
  error,
  className,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & {
  label: string;
  detail?: string;
  error?: string;
}) {
  return (
    <LaunchField label={label} detail={detail} error={error}>
      <textarea aria-label={label} className={cn(launchControlClass, 'min-h-24 resize-y', className)} {...props} />
    </LaunchField>
  );
}

export function LaunchNativeSelect({
  label,
  detail,
  value,
  options,
  onChange,
  disabled,
}: {
  label: string;
  detail?: string;
  value: string;
  options: LaunchSelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<'down' | 'up'>('down');
  const rootRef = useRef<HTMLDivElement>(null);
  const selected = options.find((option) => option.value === value);
  const isDisabled = disabled || options.length === 0;

  useEffect(() => {
    if (!open) return;

    function onPointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') setOpen(false);
    }

    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  useLayoutEffect(() => {
    if (!open) return;
    const rect = rootRef.current?.getBoundingClientRect();
    if (!rect) return;

    const estimatedMenuHeight = Math.min(288, (options.length * 56) + 12);
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    setPlacement(spaceBelow < estimatedMenuHeight + 12 && spaceAbove > spaceBelow ? 'up' : 'down');
  }, [open, options.length]);

  return (
    <div ref={rootRef} className="relative space-y-2">
      <span className="flex items-center justify-between gap-2">
        <span className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">{label}</span>
        {detail ? <span className="font-data text-xs font-medium text-[var(--sandbox-console-subtle)]">{detail}</span> : null}
      </span>
      <button
        type="button"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={isDisabled}
        onClick={() => setOpen((current) => !current)}
        className={cn(
          'group flex min-h-11 w-full items-center justify-between gap-3 rounded-[5px] border px-3.5 py-2.5 text-left font-data text-[15px] font-medium shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color] duration-150 disabled:cursor-not-allowed disabled:opacity-60',
          open
            ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-control-hover)] text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow-focus)]'
            : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-text)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)]',
        )}
      >
        {selected ? (
          <SelectOptionVisual option={selected} />
        ) : (
          <span className="min-w-0 truncate">{value || 'Select option'}</span>
        )}
        <span className={cn('i-ph:caret-down shrink-0 text-sm text-[var(--sandbox-console-muted)] transition-transform group-hover:text-[var(--sandbox-console-text)]', open && 'rotate-180 text-[var(--sandbox-console-brand)]')} />
      </button>
      {open ? (
        <div
          role="listbox"
          aria-label={label}
          className={cn(
            'absolute left-0 right-0 z-[70] max-h-72 overflow-y-auto rounded-[5px] border border-[var(--sandbox-console-menu-border)] bg-[var(--sandbox-console-menu)] p-1.5 shadow-[var(--sandbox-console-menu-shadow)]',
            placement === 'up' ? 'bottom-full mb-2' : 'top-full mt-2',
          )}
        >
          {options.map((option) => {
            const active = option.value === value;
            return (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={active}
                aria-label={option.label}
                onClick={() => {
                  onChange(option.value);
                  setOpen(false);
                }}
                className={cn(
                  'flex w-full items-center justify-between gap-3 rounded-[4px] px-3 py-2.5 text-left font-display text-[15px] font-semibold transition-[background-color,color,box-shadow] duration-150',
                  active
                    ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
                    : 'text-[var(--sandbox-console-secondary)] hover:bg-[var(--sandbox-console-menu-strong)] hover:text-[var(--sandbox-console-text)] hover:shadow-[inset_3px_0_0_var(--sandbox-console-border-hover)]',
                )}
              >
                <SelectOptionVisual option={option} />
                {active ? <span className="i-ph:check-bold shrink-0 text-xs text-[var(--sandbox-console-brand)]" /> : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

export function SelectOptionVisual({ option }: { option: LaunchSelectOption }) {
  if (!option.identity) {
    return <span className="min-w-0 truncate">{option.label}</span>;
  }

  return (
    <span className="flex min-w-0 items-center gap-3">
      <IdentityMark identity={option.identity} size="sm" />
      <span className="min-w-0">
        <span className="block truncate">{option.label}</span>
        {(option.detail ?? option.identity.detail) ? (
          <span className="mt-0.5 block truncate font-data text-[11px] font-medium text-[var(--sandbox-console-subtle)]">
            {option.detail ?? option.identity.detail}
          </span>
        ) : null}
      </span>
    </span>
  );
}

export function LaunchImageSelect({
  value,
  options,
  onChange,
  placeholder,
}: {
  value: string;
  options: { label: string; value: string }[];
  onChange: (value: string) => void;
  placeholder: string;
}) {
  const selectedOption = options.find((option) => option.value === value);
  const selectOptions = [
    ...options.map((option) => ({
      ...option,
      label: formatImageOptionLabel(option.value, option.label),
      detail: getImageIdentity(option.value).detail,
      identity: getImageIdentity(option.value),
    })),
    {
      label: 'Custom image...',
      value: CUSTOM_IMAGE_VALUE,
      detail: getImageIdentity(CUSTOM_IMAGE_VALUE).detail,
      identity: getImageIdentity(CUSTOM_IMAGE_VALUE),
    },
  ];
  const selectValue = selectedOption ? selectedOption.value : CUSTOM_IMAGE_VALUE;

  return (
    <div className="space-y-2">
      <LaunchNativeSelect
        label="Docker Image"
        value={selectValue}
        options={selectOptions}
        onChange={(next) => {
          if (next === CUSTOM_IMAGE_VALUE) {
            if (selectedOption) onChange('');
            return;
          }
          onChange(next);
        }}
      />
      {selectValue === CUSTOM_IMAGE_VALUE ? (
        <LaunchInput
          label="Custom Image"
          value={selectedOption ? '' : value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          className="font-data"
        />
      ) : null}
    </div>
  );
}

export function SegmentedControl({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: { label: string; value: string }[];
  onChange: (value: string) => void;
}) {
  return (
    <div className="space-y-2">
      <p className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">{label}</p>
      <div
        className={cn(
          'grid gap-1 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] p-1 shadow-[var(--sandbox-console-control-shadow)]',
          options.length === 2 ? 'sm:grid-cols-2' : 'sm:grid-cols-3',
        )}
      >
        {options.map((option) => {
          const active = option.value === value;
          const identity = getRuntimeIdentity(option.value);
          return (
            <button
              key={option.value}
              type="button"
              onClick={() => onChange(option.value)}
              className={cn(
                'flex min-h-12 items-center justify-center gap-2 rounded-[4px] px-3 text-center font-display text-sm font-bold transition-[background-color,color,box-shadow,transform] duration-150 active:scale-[0.98]',
                active
                  ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_0_0_0_1px_var(--sandbox-console-brand-border),inset_0_3px_0_var(--sandbox-console-brand)]'
                  : 'text-[var(--sandbox-console-muted)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[inset_0_3px_0_var(--sandbox-console-border-hover)]',
              )}
            >
              <IdentityMark identity={identity} size="sm" />
              <span className="whitespace-nowrap text-[13px] sm:text-sm">{option.label.replace(' (default)', '')}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function LaunchToggle({
  label,
  detail,
  identity,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  detail?: string;
  identity?: IdentityMeta;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'group flex w-full items-center gap-3 rounded-[5px] border p-3.5 text-left shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color,transform] duration-150 active:scale-[0.99] disabled:cursor-not-allowed disabled:opacity-60',
        checked
          ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
          : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)]',
      )}
    >
      {identity ? <IdentityMark identity={identity} size="md" /> : (
        <span
          className={cn(
            'flex h-5 w-5 shrink-0 items-center justify-center rounded border transition-colors',
            checked
              ? 'border-[var(--sandbox-console-brand)] bg-[var(--sandbox-console-brand)] text-white'
              : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] text-transparent',
          )}
        >
          <span className="i-ph:check-bold text-xs" />
        </span>
      )}
      <span className="min-w-0">
        <span className="block font-display text-base font-bold tracking-tight text-[var(--sandbox-console-text)]">{label}</span>
        {detail ? <span className="mt-0.5 block text-sm leading-6 text-[var(--sandbox-console-muted)] group-hover:text-[var(--sandbox-console-secondary)]">{detail}</span> : null}
      </span>
    </button>
  );
}

export function ResourceSizingControls({
  job,
  values,
  onChange,
}: {
  job: JobDefinition;
  values: Record<string, unknown>;
  onChange: (name: string, value: unknown) => void;
}) {
  return (
    <div className="space-y-2">
      <p className="font-display text-sm font-bold text-[var(--sandbox-console-secondary)]">Resources</p>
      <div className="grid grid-cols-3 gap-2">
        <ResourceNumberInput
          label="CPU Cores"
          shortLabel="CPU"
          unit="cores"
          identity={getResourceIdentity('cpu')}
          field={field(job, 'cpuCores')}
          value={valueNumber(values, 'cpuCores', 2)}
          onChange={(value) => onChange('cpuCores', value)}
        />
        <ResourceNumberInput
          label="Memory (MB)"
          shortLabel="RAM"
          unit="MB"
          identity={getResourceIdentity('memory')}
          field={field(job, 'memoryMb')}
          value={valueNumber(values, 'memoryMb', 2048)}
          onChange={(value) => onChange('memoryMb', value)}
        />
        <ResourceNumberInput
          label="Disk (GB)"
          shortLabel="Disk"
          unit="GB"
          identity={getResourceIdentity('disk')}
          field={field(job, 'diskGb')}
          value={valueNumber(values, 'diskGb', 10)}
          onChange={(value) => onChange('diskGb', value)}
        />
      </div>
    </div>
  );
}

export function ResourceNumberInput({
  label,
  shortLabel,
  unit,
  identity,
  field: fieldDef,
  value,
  onChange,
}: {
  label: string;
  shortLabel: string;
  unit: string;
  identity: IdentityMeta;
  field?: JobFieldDef;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="group block min-w-0 cursor-text rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] p-3 shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,transform] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] focus-within:border-[var(--sandbox-console-brand-border)] focus-within:bg-[var(--sandbox-console-control-hover)] focus-within:shadow-[var(--sandbox-console-control-shadow-focus)]">
      <span className="flex items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-1.5">
          <IdentityMark identity={identity} size="sm" className="h-5 w-5 rounded-[4px] text-[9px]" />
          <span className="whitespace-nowrap font-display text-[11px] font-bold uppercase tracking-[0.05em] text-[var(--sandbox-console-muted)] group-hover:text-[var(--sandbox-console-secondary)]">{shortLabel}</span>
        </span>
        <span className="i-ph:pencil-simple-line hidden shrink-0 text-sm text-[var(--sandbox-console-brand)] opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 sm:inline-block" />
      </span>
      <span className="mt-1.5 flex min-w-0 items-baseline gap-1.5">
        <input
          aria-label={label}
          type="number"
          min={fieldDef?.min}
          max={fieldDef?.max}
          step={fieldDef?.step ?? 1}
          value={value}
          onChange={(event) => onChange(clampNumber(Number(event.target.value), fieldDef?.min, fieldDef?.max))}
          className="min-w-0 flex-1 bg-transparent font-data text-xl font-bold leading-none text-[var(--sandbox-console-text)] outline-none"
        />
        <span className="shrink-0 font-data text-[11px] font-bold uppercase text-[var(--sandbox-console-subtle)]">{unit}</span>
      </span>
    </label>
  );
}

export function AgentConfigurationField({
  image,
  value,
  usesBundledSelector,
  onChange,
}: {
  image: string;
  value: string;
  usesBundledSelector: boolean;
  onChange: (value: string) => void;
}) {
  const helpText = usesBundledSelector
    ? 'Choose an agent already bundled in this image. None keeps the resource compute-only and hides chat.'
    : 'Custom images must already register this agent identifier internally. Typing a new name here does not create a new agent.';
  const selectValue = value || BUNDLED_NO_AGENT_VALUE;

  return (
    <div className="border-t border-[var(--sandbox-console-border)] pt-4">
      {usesBundledSelector ? (
        <LaunchNativeSelect
          label="Agent"
          value={selectValue}
          onChange={(next) => onChange(sanitizeBundledAgentIdentifier(next))}
          options={BUNDLED_AGENT_OPTIONS.map((option) => ({
            ...option,
            detail: getAgentIdentity(option.value).detail,
            identity: getAgentIdentity(option.value),
          }))}
        />
      ) : (
        <LaunchInput
          label="Agent"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={image ? 'default' : 'Choose an image first'}
        />
      )}
      <p className="mt-2 text-sm leading-6 text-[var(--sandbox-console-muted)]">
        {helpText}
      </p>
      {!usesBundledSelector && value.trim() !== '' && (
        <div className="mt-3 rounded-[5px] border border-amber-400/25 bg-amber-400/10 px-3.5 py-2.5">
          <p className="text-sm leading-6 text-amber-200">
            Custom agent identifiers depend on the selected image registering the agent
            internally. If the image does not recognize this name, chat will fail after provision.
          </p>
        </div>
      )}
    </div>
  );
}

export function AllHarnessCapabilityField({
  enabled,
  onChange,
}: {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <LaunchToggle
      label="All-Harness Runtime"
      checked={enabled}
      onChange={onChange}
      identity={getCapabilityIdentity('harness')}
      detail="Request Claude, Codex, opencode, Kimi, and Gemini harnesses in the sidecar image."
    />
  );
}

export function ComputerUseCapabilityField({
  enabled,
  onChange,
}: {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <LaunchToggle
      label="Computer Use"
      checked={enabled}
      onChange={onChange}
      identity={getCapabilityIdentity('computer-use')}
      detail="Enable browser/computer-use tools for visual agent tasks when the sidecar image supports them."
    />
  );
}
