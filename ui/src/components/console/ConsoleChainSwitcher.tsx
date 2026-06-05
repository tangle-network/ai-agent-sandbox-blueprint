import { useEffect, useRef, useState } from 'react';
import { useStore } from '@nanostores/react';
import {
  getNetworks,
  selectedChainIdStore,
} from '@tangle-network/blueprint-ui';
import { cn } from '@tangle-network/blueprint-ui';

type Placement = 'up' | 'down';

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

export function ConsoleChainSwitcher({ placement = 'down' }: { placement?: Placement }) {
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
        className="group flex h-9 w-full min-w-0 items-center justify-center gap-1.5 rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)] px-2 font-data text-xs font-medium text-[var(--sandbox-console-secondary)] transition-colors hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-panel-strong)]"
        title={current?.label ?? 'Select network'}
        aria-expanded={open}
        aria-haspopup="menu"
      >
        <span className={cn('shrink-0 text-sm text-[var(--sandbox-console-success)]', chainIcon(current?.label, current?.chain?.name))} />
        <span className="min-w-0 truncate">{current?.shortLabel ?? 'Network'}</span>
        <span className={cn('shrink-0 text-[10px] text-[var(--sandbox-console-muted)] transition-transform', open && 'rotate-180', placement === 'up' && 'rotate-180', open && placement === 'up' && 'rotate-0', 'i-ph:caret-down')} />
      </button>

      {open ? (
        <div
          className={cn(
            'absolute z-50 w-56 overflow-hidden rounded-md border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel-strong)] py-1 shadow-[var(--sandbox-console-shadow-lg)]',
            placement === 'up' ? 'bottom-full left-0 mb-2' : 'right-0 top-full mt-2',
          )}
          role="menu"
        >
          <div className="px-3 py-2 font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
            Network
          </div>
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
                  'flex w-full items-center gap-2.5 px-3 py-2.5 text-left text-sm transition-colors',
                  selected
                    ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)]'
                    : 'text-[var(--sandbox-console-secondary)] hover:bg-[var(--sandbox-console-hover)] hover:text-[var(--sandbox-console-text)]',
                )}
                role="menuitemradio"
                aria-checked={selected}
              >
                <span className={cn('text-base', selected ? 'text-[var(--sandbox-console-brand)]' : 'text-[var(--sandbox-console-muted)]', chainIcon(network.label, network.chain.name))} />
                <span className="min-w-0 flex-1 truncate font-display font-medium">{network.label}</span>
                {selected ? <span className="i-ph:check-bold text-xs text-[var(--sandbox-console-brand)]" /> : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
