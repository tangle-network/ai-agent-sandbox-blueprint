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
    <span className="flex min-w-0 items-center gap-2.5">
      <TangleBrandLogo />
      <span className="min-w-0 truncate font-display text-base font-bold tracking-tight text-[var(--sandbox-console-text)]">
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
      <aside className="hidden w-[268px] shrink-0 border-r border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-rail)] lg:flex lg:flex-col">
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
                  'group flex h-11 items-center gap-3 rounded-[5px] border px-3.5 font-display text-[15px] font-semibold transition-[background-color,border-color,color,box-shadow,transform] duration-150 active:scale-[0.99]',
                  active
                    ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
                    : 'border-transparent text-[var(--sandbox-console-muted)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[inset_3px_0_0_var(--sandbox-console-border-hover)]',
                )}
              >
                <span className={cn('text-lg', item.icon)} />
                <span>{item.label}</span>
              </Link>
            );
          })}
        </nav>

        <div className="shrink-0 border-t border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] p-3">
          <div className="space-y-3">
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
        <header className="flex h-16 shrink-0 items-center justify-between border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-bg)] px-3 lg:hidden">
          <div ref={mobileMenuRef} className="relative">
            <button
              type="button"
              onClick={() => setMobileNavOpen((open) => !open)}
              className="flex h-10 w-10 items-center justify-center rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-text)] shadow-[var(--sandbox-console-control-shadow)] transition-colors hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)]"
              aria-label="Navigation menu"
            >
              <span className={cn('text-lg', mobileNavOpen ? 'i-ph:x' : 'i-ph:list')} />
            </button>
            {mobileNavOpen ? (
              <nav className="absolute left-0 top-12 z-50 w-64 overflow-hidden rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel-strong)] shadow-[var(--sandbox-console-shadow-lg)]">
                {navItems.map((item) => {
                  const active = isActivePath(location.pathname, item.href);
                  return (
                    <Link
                      key={item.href}
                      to={item.href}
                      className={cn(
                        'flex items-center gap-3 border-b border-[var(--sandbox-console-border)] px-3 py-3 font-display text-[15px] font-semibold transition-colors',
                        active
                          ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)]'
                          : 'text-[var(--sandbox-console-muted)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)]',
                      )}
                    >
                      <span className={cn('text-lg', item.icon)} />
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
