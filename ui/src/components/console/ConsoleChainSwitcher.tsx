import { useEffect, useRef, useState } from 'react';
import { useStore } from '@nanostores/react';
import {
  getNetworks,
  selectedChainIdStore,
} from '@tangle-network/blueprint-ui';
import { cn } from '@tangle-network/blueprint-ui';

type Placement = 'up' | 'down';
type Align = 'start' | 'end';

interface ConsoleChainSwitcherProps {
  placement?: Placement;
  align?: Align;
  compact?: boolean;
}

function chainIcon(label: string | undefined, chainName: string | undefined): string {
  if (label === 'Base Sepolia' || chainName === 'Base Sepolia') return 'i-ph:hexagon';
  if (label === 'Tangle Local' || chainName === 'Tangle Local') return 'i-ph:desktop';
  if (label === 'Tangle Testnet' || chainName === 'Tangle Testnet') return 'i-ph:flask';
  if (label === 'Tangle Mainnet' || chainName === 'Tangle') return 'i-ph:globe-hemisphere-west';
  return 'i-ph:globe';
}

function orderedChainIds(): number[] {
  const priority: Record<string, number> = {
    'Base Sepolia': 0,
    'Tangle Testnet': 1,
    'Tangle Mainnet': 2,
    Tangle: 2,
    'Tangle Local': 9,
  };

  return Object.entries(getNetworks())
    .sort(([, a], [, b]) => {
      const aPriority = priority[a?.label ?? a?.chain?.name ?? ''] ?? 99;
      const bPriority = priority[b?.label ?? b?.chain?.name ?? ''] ?? 99;
      if (aPriority !== bPriority) return aPriority - bPriority;
      return a.chain.id - b.chain.id;
    })
    .map(([chainId]) => Number(chainId));
}

export function ConsoleChainSwitcher({
  placement = 'down',
  align = 'end',
  compact = false,
}: ConsoleChainSwitcherProps = {}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selectedChainId = useStore(selectedChainIdStore);
  const current = getNetworks()[selectedChainId];

  useEffect(() => {
    if (!open) return;

    function onPointerDown(event: MouseEvent) {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') setOpen(false);
    }

    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  function selectChain(chainId: number) {
    selectedChainIdStore.set(chainId);
    setOpen(false);
    window.location.reload();
  }

  return (
    <div ref={ref} className="relative min-w-0">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className={cn(
          'group inline-flex h-10 max-w-full items-center justify-center gap-2 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-2 font-display text-sm font-bold text-[var(--sandbox-console-secondary)] shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color,transform] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] active:scale-[0.98] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60',
          compact ? 'w-10 min-w-0 px-0' : 'w-full min-w-0',
        )}
        title={compact ? (current?.label ?? 'Select network') : undefined}
        aria-label="Network"
        aria-expanded={open}
        aria-haspopup="menu"
      >
        <span className={cn('shrink-0 text-base text-[var(--sandbox-console-success)]', chainIcon(current?.label, current?.chain?.name))} />
        {!compact ? <span className="min-w-0 truncate">{current?.label ?? 'Network'}</span> : null}
        {!compact ? (
          <span className={cn('i-ph:caret-up-down shrink-0 text-xs text-[var(--sandbox-console-muted)] transition-colors group-hover:text-[var(--sandbox-console-text)]', open && 'text-[var(--sandbox-console-brand)]')} />
        ) : null}
      </button>

      {open ? (
        <div
          className={cn(
            'absolute z-50 max-h-[min(24rem,calc(100vh-1rem))] w-[min(18rem,calc(100vw-1rem))] overflow-hidden rounded-[5px] border border-[var(--sandbox-console-menu-border)] bg-[var(--sandbox-console-menu)] p-1.5 shadow-[var(--sandbox-console-menu-shadow)]',
            align === 'start' ? 'left-0' : 'right-0',
            placement === 'up' ? 'bottom-full mb-2' : 'top-full mt-2',
          )}
          role="menu"
        >
          <div className="px-2 py-1.5 font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
            Network
          </div>
          <div className="max-h-[17rem] overflow-y-auto [scrollbar-gutter:stable]">
            {orderedChainIds().map((chainId) => {
              const network = getNetworks()[chainId];
              if (!network) return null;
              const selected = chainId === selectedChainId;

              return (
                <button
                  key={chainId}
                  type="button"
                  onClick={() => selectChain(chainId)}
                  className={cn(
                    'grid w-full grid-cols-[1.25rem_minmax(0,1fr)_auto] items-center gap-2 rounded-[5px] px-2 py-2 text-left transition-[background-color,color,box-shadow] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60',
                    selected
                      ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
                      : 'text-[var(--sandbox-console-secondary)] hover:bg-[var(--sandbox-console-menu-strong)] hover:text-[var(--sandbox-console-text)] hover:shadow-[inset_3px_0_0_var(--sandbox-console-border-hover)]',
                  )}
                  role="menuitemradio"
                  aria-checked={selected}
                >
                  <span className={cn('text-base', selected ? 'text-[var(--sandbox-console-brand)]' : 'text-[var(--sandbox-console-muted)]', chainIcon(network.label, network.chain.name))} />
                  <span className="min-w-0 truncate font-display text-sm font-bold">{network.label}</span>
                  <span className="font-data text-[10px] font-semibold tabular-nums text-[var(--sandbox-console-muted)]">
                    {network.chain.id}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      ) : null}
    </div>
  );
}
