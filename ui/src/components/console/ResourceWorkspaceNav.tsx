import { Link } from 'react-router';
import { cn } from '@tangle-network/blueprint-ui';

export type WorkspaceNavItem = {
  label: string;
  href: string;
  icon: string;
  disabled?: boolean;
};

export function ResourceWorkspaceNav({
  items,
  activePath,
}: {
  items: WorkspaceNavItem[];
  activePath?: string;
}) {
  return (
    <nav className="mb-4 overflow-x-auto rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] p-1">
      <div className="flex min-w-max items-center gap-1">
        {items.filter((item) => !item.disabled).map((item) => {
          const active = activePath === item.href || activePath?.startsWith(`${item.href}/`);
          return (
            <Link
              key={item.href}
              to={item.href}
              className={cn(
                'flex h-9 items-center gap-2 rounded px-3 font-data text-[11px] uppercase tracking-[0.08em] transition-colors',
                active
                  ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-brand)]'
                  : 'text-[var(--sandbox-console-muted)] hover:bg-[var(--sandbox-console-surface)] hover:text-[var(--sandbox-console-text)]',
              )}
            >
              <span className={cn('text-sm', item.icon)} />
              {item.label}
            </Link>
          );
        })}
      </div>
    </nav>
  );
}
