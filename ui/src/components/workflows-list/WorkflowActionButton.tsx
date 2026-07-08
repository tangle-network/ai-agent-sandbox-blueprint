import { type ReactNode } from 'react';
import { cn } from '@tangle-network/blueprint-ui';

export function WorkflowActionButton({
  children,
  disabled,
  icon,
  onClick,
  tone = 'secondary',
}: {
  children: ReactNode;
  disabled?: boolean;
  icon?: string;
  onClick?: () => void;
  tone?: 'secondary' | 'success';
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'inline-flex h-8 items-center justify-center gap-1.5 rounded-[4px] border px-2.5 font-display text-xs font-bold transition-[background-color,border-color,box-shadow,color,transform] duration-150 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-55',
        tone === 'success'
          ? 'border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)] hover:border-[var(--sandbox-console-success)] hover:bg-[color-mix(in_srgb,var(--sandbox-console-success)_18%,transparent)] hover:shadow-[inset_3px_0_0_var(--sandbox-console-success)]'
          : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[var(--sandbox-console-control-shadow-hover)]',
      )}
    >
      {icon ? <span className={cn('text-xs', icon)} /> : null}
      {children}
    </button>
  );
}
