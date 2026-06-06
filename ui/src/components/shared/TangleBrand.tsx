import { cn } from '@tangle-network/blueprint-ui';
import { useEffect, useState } from 'react';

function useDocumentTheme() {
  const [theme, setTheme] = useState<'dark' | 'light'>(() => {
    if (typeof document === 'undefined') return 'dark';
    return document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
  });

  useEffect(() => {
    const root = document.documentElement;
    const sync = () => setTheme(root.dataset.theme === 'light' ? 'light' : 'dark');
    sync();

    const observer = new MutationObserver(sync);
    observer.observe(root, { attributes: true, attributeFilter: ['data-theme'] });
    return () => observer.disconnect();
  }, []);

  return theme;
}

export function TangleBrandLogo({ compact = false, className }: { compact?: boolean; className?: string }) {
  const theme = useDocumentTheme();
  const src = compact ? '/tangle-mark.svg' : theme === 'dark' ? '/tangle-logo-light.svg' : '/tangle-logo.svg';

  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center justify-center overflow-hidden',
        compact ? 'h-8 w-8' : 'h-9 w-[124px]',
        className,
      )}
      aria-hidden="true"
    >
      <img src={src} alt="" className="h-full w-full object-contain" />
    </span>
  );
}

export function TangleOperatorMark({ label }: { label?: string }) {
  return (
    <span
      className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-[4px] border border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] p-1"
      title={label}
      aria-hidden="true"
    >
      <img src="/tangle-mark.svg" alt="" className="h-full w-full object-contain" />
    </span>
  );
}
