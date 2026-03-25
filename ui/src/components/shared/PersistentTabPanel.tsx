import type { ReactNode } from 'react';
import { cn } from '@tangle-network/blueprint-ui';

interface PersistentTabPanelProps {
  active: boolean;
  children: ReactNode;
  className?: string;
}

export function PersistentTabPanel({
  active,
  children,
  className,
}: PersistentTabPanelProps) {
  return (
    <div
      role="tabpanel"
      aria-hidden={!active}
      hidden={!active}
      data-state={active ? 'active' : 'inactive'}
      className={cn(!active && 'hidden', className)}
    >
      {children}
    </div>
  );
}
