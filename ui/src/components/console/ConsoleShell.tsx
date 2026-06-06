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

const SIDEBAR_COLLAPSED_KEY = 'sandbox:console-sidebar-collapsed';

function BrandMark({ compact = false }: { compact?: boolean }) {
  return (
    <span className="flex min-w-0 items-center justify-center">
      <TangleBrandLogo compact={compact} />
    </span>
  );
}

function isActivePath(pathname: string, href: string) {
  return href === '/' ? pathname === '/' : pathname.startsWith(href);
}

const dockControlClass = [
  'min-w-0',
  '[&>button]:!h-10',
  '[&>button]:!w-full',
  '[&>button]:!justify-center',
  '[&>button]:!rounded-[5px]',
  '[&>button]:!border',
  '[&>button]:!border-[var(--sandbox-console-border)]',
  '[&>button]:!bg-[var(--sandbox-console-control)]',
  '[&>button]:!px-2.5',
  '[&>button]:!text-[var(--sandbox-console-secondary)]',
  '[&>button]:!shadow-[var(--sandbox-console-control-shadow)]',
  '[&>button]:transition-[background-color,border-color,box-shadow,color,opacity,transform]',
  '[&>button]:duration-150',
  '[&>button:hover]:!border-[var(--sandbox-console-border-hover)]',
  '[&>button:hover]:!bg-[var(--sandbox-console-control-hover)]',
  '[&>button:hover]:!text-[var(--sandbox-console-text)]',
  '[&>button:hover]:!shadow-[var(--sandbox-console-control-shadow-hover)]',
  '[&>div>button]:!h-10',
  '[&>div>button]:!w-full',
  '[&>div>button]:!justify-center',
  '[&>div>button]:!rounded-[5px]',
  '[&>div>button]:!border',
  '[&>div>button]:!border-[var(--sandbox-console-border)]',
  '[&>div>button]:!bg-[var(--sandbox-console-control)]',
  '[&>div>button]:!px-2.5',
  '[&>div>button]:!text-[var(--sandbox-console-secondary)]',
  '[&>div>button]:!shadow-[var(--sandbox-console-control-shadow)]',
  '[&>div>button]:transition-[background-color,border-color,box-shadow,color,opacity,transform]',
  '[&>div>button]:duration-150',
  '[&>div>button:hover]:!border-[var(--sandbox-console-border-hover)]',
  '[&>div>button:hover]:!bg-[var(--sandbox-console-control-hover)]',
  '[&>div>button:hover]:!text-[var(--sandbox-console-text)]',
  '[&>div>button:hover]:!shadow-[var(--sandbox-console-control-shadow-hover)]',
].join(' ');

const dockIconControlClass = [
  'flex h-10 min-w-0 items-center justify-center rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)] shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color,transform] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] active:scale-[0.98]',
  '[&>button]:!h-9',
  '[&>button]:!w-9',
  '[&>button]:!min-w-0',
  '[&>button]:!overflow-hidden',
  '[&>button]:!rounded-[5px]',
  '[&>button]:!border-0',
  '[&>button]:!bg-transparent',
  '[&>button]:!p-0',
  '[&>button]:!shadow-none',
  '[&>button]:!text-[var(--sandbox-console-secondary)]',
  '[&>div>button]:!h-9',
  '[&>div>button]:!w-9',
  '[&>div>button]:!min-w-0',
  '[&>div>button]:!overflow-hidden',
  '[&>div>button]:!rounded-[5px]',
  '[&>div>button]:!border-0',
  '[&>div>button]:!bg-transparent',
  '[&>div>button]:!p-0',
  '[&>div>button]:!shadow-none',
  '[&>div>button]:!text-[var(--sandbox-console-secondary)]',
].join(' ');

function SidebarIconButton({ label, icon, onClick }: { label: string; icon: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-[5px] text-[var(--sandbox-console-muted)] transition-[background-color,color,transform] duration-150 hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60"
      aria-label={label}
      title={label}
    >
      <span className={cn('text-lg', icon)} aria-hidden="true" />
    </button>
  );
}

function ExpandedCommandDock() {
  return (
    <div className="border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] p-1.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.055)]">
      <div className={dockControlClass}>
        <WalletButton align="start" side="up" />
      </div>
      <div className={cn('mt-1.5', dockControlClass)}>
        <ConsoleChainSwitcher align="start" placement="up" />
      </div>
      <div className="mt-1.5 grid grid-cols-2 gap-1.5">
        <div className={dockIconControlClass}>
          <TxDropdown compact align="start" side="up" />
        </div>
        <div className={dockIconControlClass}>
          <ThemeToggle />
        </div>
      </div>
    </div>
  );
}

function CollapsedCommandDock() {
  return (
    <>
      <div className={dockIconControlClass}>
        <ConsoleChainSwitcher compact align="start" placement="up" />
      </div>
      <div className={dockIconControlClass}>
        <WalletButton compact align="start" side="up" />
      </div>
    </>
  );
}

export function ConsoleShell({ children }: { children: ReactNode }) {
  const location = useLocation();
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    if (typeof window === 'undefined') return false;
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === 'true';
  });
  const mobileMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, sidebarCollapsed ? 'true' : 'false');
  }, [sidebarCollapsed]);

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
      <aside
        className={cn(
          'relative z-40 hidden shrink-0 flex-col border-r border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-rail)] transition-[width] duration-200 lg:flex',
          sidebarCollapsed ? 'w-16' : 'w-[268px]',
        )}
      >
        <div
          className={cn(
            'flex h-16 shrink-0 items-center border-b border-[var(--sandbox-console-border)]',
            sidebarCollapsed ? 'justify-center px-2' : 'justify-between gap-2 px-3',
          )}
        >
          {sidebarCollapsed ? (
            <div className="group/brand relative h-11 w-11">
              <Link
                to="/"
                className="inline-flex h-11 w-11 min-w-0 items-center justify-center rounded-[5px] text-[var(--sandbox-console-text)] transition-colors hover:bg-[var(--sandbox-console-control-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60"
                aria-label="Tangle Sandbox"
                title="Tangle Sandbox"
              >
                <BrandMark compact />
              </Link>
              <button
                type="button"
                onClick={() => setSidebarCollapsed(false)}
                className="pointer-events-none absolute inset-0 inline-flex h-11 w-11 items-center justify-center rounded-[5px] border border-[var(--sandbox-console-border-hover)] bg-[var(--sandbox-console-panel-strong)] text-[var(--sandbox-console-text)] opacity-0 shadow-[0_10px_24px_rgba(0,0,0,0.22)] transition-[opacity,background-color,border-color,color,transform] duration-150 hover:bg-[var(--sandbox-console-brand-soft)] active:scale-95 focus-visible:pointer-events-auto focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60 group-hover/brand:pointer-events-auto group-hover/brand:opacity-100"
                aria-label="Expand sidebar"
                title="Expand sidebar"
              >
                <span className="i-ph:caret-right-bold text-lg" aria-hidden="true" />
              </button>
            </div>
          ) : (
            <Link
              to="/"
              className="inline-flex h-11 min-w-0 items-center gap-2 rounded-[5px] px-2 text-[var(--sandbox-console-text)] transition-colors hover:bg-[var(--sandbox-console-control-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60"
              aria-label="Tangle Sandbox"
            >
              <BrandMark />
            </Link>
          )}
          {!sidebarCollapsed ? (
            <SidebarIconButton
              label="Collapse sidebar"
              onClick={() => setSidebarCollapsed(true)}
              icon="i-ph:caret-left-bold"
            />
          ) : null}
        </div>

        <nav
          className={cn(
            'flex-1 space-y-1 overflow-y-auto py-4',
            sidebarCollapsed ? 'px-2' : 'px-3',
          )}
          aria-label="Tangle sandbox navigation"
        >
          {navItems.map((item) => {
            const active = isActivePath(location.pathname, item.href);
            return (
              <Link
                key={item.href}
                to={item.href}
                className={cn(
                  'group relative flex h-11 items-center rounded-[5px] border font-display text-[15px] font-bold transition-[background-color,border-color,color,box-shadow,transform] duration-150 active:scale-[0.98]',
                  sidebarCollapsed ? 'w-11 justify-center px-0' : 'gap-3 px-3.5',
                  active
                    ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
                    : 'border-transparent text-[var(--sandbox-console-muted)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[inset_3px_0_0_var(--sandbox-console-border-hover)]',
                )}
                title={sidebarCollapsed ? item.label : undefined}
                aria-label={sidebarCollapsed ? item.label : undefined}
              >
                <span className={cn('shrink-0 text-lg transition-colors', active ? 'text-[var(--sandbox-console-brand)]' : 'text-[var(--sandbox-console-subtle)] group-hover:text-[var(--sandbox-console-secondary)]', item.icon)} />
                {!sidebarCollapsed ? <span className="truncate">{item.label}</span> : null}
              </Link>
            );
          })}
        </nav>

        <div
          className={cn(
            'shrink-0 border-t border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-rail)]',
            sidebarCollapsed ? 'flex flex-col items-center gap-1.5 p-2' : 'p-2',
          )}
        >
          {sidebarCollapsed ? <CollapsedCommandDock /> : <ExpandedCommandDock />}
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
              <nav className="absolute left-0 top-12 z-50 w-64 overflow-hidden rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel-strong)] shadow-[var(--sandbox-console-shadow-lg)]" aria-label="Tangle sandbox navigation">
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
                  <ConsoleChainSwitcher placement="down" align="start" />
                  <TxDropdown compact align="start" />
                  <ThemeToggle />
                </div>
              </nav>
            ) : null}
          </div>

          <Link to="/" className="flex items-center" aria-label="Tangle Sandbox Console">
            <BrandMark />
          </Link>

          <WalletButton compact />
        </header>

        <main className="min-h-0 flex-1 overflow-hidden">
          {children}
        </main>
      </div>
    </div>
  );
}
