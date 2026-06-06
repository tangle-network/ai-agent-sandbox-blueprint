import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Link, useLocation } from 'react-router';
import {
  ThemeToggle,
} from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';
import { TxDropdown } from '~/components/layout/TxDropdown';
import { WalletButton } from '~/components/layout/WalletButton';
import { ConsoleChainSwitcher } from '~/components/console/ConsoleChainSwitcher';
import { TangleBrandLogo } from '~/components/shared/TangleBrand';

const navItems = [
  { label: 'Fleet', href: '/', icon: 'i-ph:activity' },
  { label: 'Sandboxes', href: '/sandboxes', icon: 'i-ph:hard-drives' },
  { label: 'Instances', href: '/instances', icon: 'i-ph:cube' },
  { label: 'Automation', href: '/workflows', icon: 'i-ph:flow-arrow' },
  { label: 'Activity', href: '/activity', icon: 'i-ph:pulse' },
  { label: 'Operators', href: '/operators', icon: 'i-ph:users-three' },
  { label: 'Launch', href: '/create', icon: 'i-ph:plus-circle' },
];

function BrandMark() {
  return (
    <span className="flex min-w-0 items-center gap-2">
      <TangleBrandLogo />
      <span className="min-w-0 truncate font-display text-sm font-semibold text-[var(--sandbox-console-text)]">
        Sandbox
      </span>
    </span>
  );
}

function isActivePath(pathname: string, href: string) {
  return href === '/' ? pathname === '/' : pathname.startsWith(href);
}

export function ConsoleShell({ children }: { children: ReactNode }) {
  const location = useLocation();
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const mobileMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setMobileNavOpen(false);
  }, [location.pathname]);

  useEffect(() => {
    if (!mobileNavOpen) return;

    function onPointerDown(event: MouseEvent) {
      if (mobileMenuRef.current && !mobileMenuRef.current.contains(event.target as Node)) {
        setMobileNavOpen(false);
      }
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') setMobileNavOpen(false);
    }

    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [mobileNavOpen]);

  return (
    <div className="sandbox-console flex h-[100dvh] overflow-hidden bg-[var(--sandbox-console-bg)] text-[var(--sandbox-console-text)]">
      <aside className="hidden w-[248px] shrink-0 border-r border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-rail)] lg:flex lg:flex-col">
        <div className="flex h-16 shrink-0 items-center border-b border-[var(--sandbox-console-border)] px-4">
          <Link to="/" className="flex min-w-0 items-center" aria-label="Tangle Sandbox Console">
            <BrandMark />
          </Link>
        </div>

        <nav className="flex-1 space-y-1 overflow-y-auto px-3 py-4">
          {navItems.map((item) => {
            const active = isActivePath(location.pathname, item.href);
            return (
              <Link
                key={item.href}
                to={item.href}
                className={cn(
                  'group flex h-10 items-center gap-3 rounded-md border px-3 text-sm font-medium transition-colors',
                  active
                    ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)]'
                    : 'border-transparent text-[var(--sandbox-console-muted)] hover:border-[var(--sandbox-console-border)] hover:bg-[var(--sandbox-console-surface)] hover:text-[var(--sandbox-console-text)]',
                )}
              >
                <span className={cn('text-base', item.icon)} />
                <span>{item.label}</span>
              </Link>
            );
          })}
        </nav>

        <div className="shrink-0 border-t border-[var(--sandbox-console-border)] p-3">
          <div className="sandbox-console-panel space-y-3 rounded-md p-3">
            <div className="grid grid-cols-3 gap-2">
              <div className="min-w-0">
                <ConsoleChainSwitcher placement="up" />
              </div>
              <TxDropdown />
              <ThemeToggle />
            </div>
            <WalletButton />
          </div>
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-14 shrink-0 items-center justify-between border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-bg)] px-3 lg:hidden">
          <div ref={mobileMenuRef} className="relative">
            <button
              type="button"
              onClick={() => setMobileNavOpen((open) => !open)}
              className="flex h-9 w-9 items-center justify-center rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] text-[var(--sandbox-console-text)]"
              aria-label="Navigation menu"
            >
              <span className={cn('text-lg', mobileNavOpen ? 'i-ph:x' : 'i-ph:list')} />
            </button>
            {mobileNavOpen ? (
              <nav className="absolute left-0 top-11 z-50 w-64 overflow-hidden rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] shadow-[var(--sandbox-console-shadow-lg)]">
                {navItems.map((item) => {
                  const active = isActivePath(location.pathname, item.href);
                  return (
                    <Link
                      key={item.href}
                      to={item.href}
                      className={cn(
                        'flex items-center gap-3 border-b border-[var(--sandbox-console-border)] px-3 py-3 text-sm',
                        active
                          ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)]'
                          : 'text-[var(--sandbox-console-muted)]',
                      )}
                    >
                      <span className={cn('text-base', item.icon)} />
                      {item.label}
                    </Link>
                  );
                })}
                <div className="flex items-center gap-2 p-3">
                  <ConsoleChainSwitcher placement="down" />
                  <TxDropdown />
                  <ThemeToggle />
                </div>
              </nav>
            ) : null}
          </div>

          <Link to="/" className="flex items-center" aria-label="Tangle Sandbox Console">
            <BrandMark />
          </Link>

          <WalletButton />
        </header>

        <main className="min-h-0 flex-1 overflow-hidden">
          {children}
        </main>
      </div>
    </div>
  );
}
