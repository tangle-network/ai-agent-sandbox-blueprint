import { useState, useEffect, useRef, useCallback } from 'react';
import { Link, useLocation } from 'react-router';
import { ChainSwitcher, ThemeToggle } from '@tangle-network/blueprint-ui/components';
import { TxDropdown } from './TxDropdown';
import { WalletButton } from './WalletButton';
import { TangleLogo } from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';

const navItems = [
  { label: 'Dashboard', href: '/', icon: 'i-ph:house' },
  { label: 'Sandboxes', href: '/sandboxes', icon: 'i-ph:hard-drives' },
  { label: 'Instances', href: '/instances', icon: 'i-ph:cube' },
  { label: 'Workflows', href: '/workflows', icon: 'i-ph:flow-arrow' },
  { label: 'Create', href: '/create', icon: 'i-ph:plus-circle' },
];

export function Header() {
  const location = useLocation();
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const toggle = useCallback(() => setOpen((v) => !v), []);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function onMouseDown(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') setOpen(false);
    }
    document.addEventListener('mousedown', onMouseDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onMouseDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  // Close on route change
  useEffect(() => { setOpen(false); }, [location.pathname]);

  return (
    <header className="fixed top-0 left-0 right-0 z-40 h-[var(--header-height)] glass-card-strong border-t-0 border-x-0 rounded-none">
      <div className="mx-auto flex h-full max-w-7xl items-center justify-between px-4 sm:px-6">
        <div className="flex items-center gap-2">
          {/* Mobile hamburger */}
          <div ref={menuRef} className="relative lg:hidden">
            <button type="button" onClick={toggle} className="p-2 rounded-lg hover:bg-cloud-elements-item-backgroundHover transition-colors" aria-label="Navigation menu">
              <div className={cn('text-lg text-cloud-elements-textSecondary transition-transform', open ? 'i-ph:x' : 'i-ph:list')} />
            </button>
            {open && (
              <nav className="absolute left-0 top-full mt-2 w-56 rounded-xl border border-cloud-elements-dividerColor/50 z-50 shadow-xl overflow-hidden py-1 bg-[var(--cloud-elements-bg-depth-2)]">
                {navItems.map((item) => (
                  <Link
                    key={item.href}
                    to={item.href}
                    className={cn(
                      'flex items-center gap-3 px-4 py-2.5 text-sm font-display font-medium transition-colors',
                      (item.href === '/' ? location.pathname === '/' : location.pathname.startsWith(item.href))
                        ? 'text-violet-700 dark:text-violet-400 bg-violet-500/10'
                        : 'text-cloud-elements-textSecondary hover:text-cloud-elements-textPrimary hover:bg-cloud-elements-item-backgroundHover',
                    )}
                  >
                    <div className={cn('text-base', item.icon)} />
                    {item.label}
                  </Link>
                ))}
                {/* Secondary actions in mobile menu */}
                <div className="border-t border-cloud-elements-dividerColor/30 mt-1 pt-1 px-3 py-2 flex items-center gap-2">
                  <ChainSwitcher />
                  <TxDropdown />
                  <ThemeToggle />
                </div>
              </nav>
            )}
          </div>

          <Link to="/" className="flex items-center group">
            <TangleLogo />
          </Link>
        </div>

        <nav className="hidden lg:flex items-center gap-1">
          {navItems.map((item) => (
            <Link
              key={item.href}
              to={item.href}
              className={cn(
                'px-4 py-2 rounded-lg text-sm font-display font-medium transition-all duration-200',
                (item.href === '/' ? location.pathname === '/' : location.pathname.startsWith(item.href))
                  ? 'text-violet-700 dark:text-violet-400 bg-violet-500/10'
                  : 'text-cloud-elements-textSecondary hover:text-cloud-elements-textPrimary hover:bg-cloud-elements-item-backgroundHover',
              )}
            >
              {item.label}
            </Link>
          ))}
        </nav>

        <div className="flex items-center gap-1.5 sm:gap-2">
          {/* Chain/Tx/Theme hidden on mobile — shown in hamburger menu instead */}
          <div className="hidden lg:flex items-center gap-2">
            <ChainSwitcher />
            <TxDropdown />
            <ThemeToggle />
          </div>
          <WalletButton />
        </div>
      </div>
    </header>
  );
}
