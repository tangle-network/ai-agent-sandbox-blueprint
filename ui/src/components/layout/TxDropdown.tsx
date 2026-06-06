import { useStore } from '@nanostores/react';
import { txListStore, pendingCount, clearTxs, type TrackedTx } from '@tangle-network/blueprint-ui';
import { cn } from '@tangle-network/blueprint-ui';
import { useDropdownMenu } from '@tangle-network/sandbox-ui/hooks';
import { timeAgo } from '@tangle-network/sandbox-ui/utils';
import type { RefObject } from 'react';

type DropdownAlign = 'start' | 'end';
type DropdownSide = 'up' | 'down';

interface TxDropdownProps {
  align?: DropdownAlign;
  side?: DropdownSide;
  compact?: boolean;
}

function StatusIcon({ status }: { status: TrackedTx['status'] }) {
  if (status === 'pending') return <div className="h-4 w-4 shrink-0 animate-spin rounded-full border-2 border-[rgba(168,123,255,0.32)] border-t-[var(--sandbox-console-brand)]" />;
  if (status === 'confirmed') return <div className="i-ph:check-circle-fill shrink-0 text-base text-[var(--sandbox-console-success)]" />;
  return <div className="i-ph:x-circle-fill shrink-0 text-base text-[var(--sandbox-console-danger)]" />;
}

export function TxDropdown({
  align = 'end',
  side = 'down',
  compact = true,
}: TxDropdownProps = {}) {
  const { open, ref, toggle, close } = useDropdownMenu({ closeOnEsc: true });
  const dropdownRef = ref as RefObject<HTMLDivElement>;
  const txs = useStore(txListStore);
  const pending = useStore(pendingCount);

  return (
    <div ref={dropdownRef} className="relative min-w-0">
      <button
        type="button"
        onClick={toggle}
        className={cn(
          'relative inline-flex h-10 max-w-full items-center justify-center gap-2 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-2 font-display text-sm font-bold text-[var(--sandbox-console-secondary)] shadow-[var(--sandbox-console-control-shadow)] transition-[background-color,border-color,box-shadow,color,transform] duration-150 hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] active:scale-[0.98] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--sandbox-console-brand)]/60',
          compact ? 'w-10 min-w-0 px-0' : 'w-full min-w-0',
        )}
        title="Transaction history"
        aria-label="Transactions"
        aria-expanded={open}
        aria-haspopup="menu"
      >
        <span className="i-ph:receipt shrink-0 text-base" aria-hidden="true" />
        {!compact ? <span className="min-w-0 truncate">Transactions</span> : null}
        {pending > 0 && (
          <span className="absolute -right-1 -top-1 flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-[var(--sandbox-console-brand)] px-1 font-data text-[10px] font-bold text-white shadow-[0_0_0_2px_var(--sandbox-console-control)] animate-pulse">{pending}</span>
        )}
      </button>

      {open && (
        <div
          className={cn(
            'absolute z-50 max-h-[min(28rem,calc(100vh-1rem))] w-[min(24rem,calc(100vw-1rem))] overflow-hidden rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel-strong)] shadow-[var(--sandbox-console-shadow-lg)]',
            align === 'start' ? 'left-0' : 'right-0',
            side === 'up' ? 'bottom-full mb-2' : 'top-full mt-2',
          )}
          role="menu"
        >
          <div className="flex items-center justify-between border-b border-[var(--sandbox-console-border)] px-4 py-3">
            <div className="flex items-center gap-2">
              <div className="i-ph:clock-counter-clockwise text-base text-[var(--sandbox-console-muted)]" />
              <span className="text-sm font-display font-bold text-[var(--sandbox-console-text)]">Transactions</span>
              {txs.length > 0 && <span className="text-xs font-data text-[var(--sandbox-console-muted)]">({txs.length})</span>}
            </div>
            {txs.length > 0 && (
              <button type="button" onClick={() => { clearTxs(); close(); }} className="text-xs font-data text-[var(--sandbox-console-muted)] transition-colors hover:text-crimson-700 dark:hover:text-crimson-400">Clear all</button>
            )}
          </div>
          <div className="max-h-[400px] overflow-y-auto">
            {txs.length === 0 ? (
              <div className="py-10 text-center">
                <div className="i-ph:receipt mx-auto mb-2 text-2xl text-[var(--sandbox-console-subtle)]" />
                <p className="text-sm text-[var(--sandbox-console-muted)]">No transactions yet</p>
              </div>
            ) : (
              txs.map((tx) => (
                <div key={tx.hash} className="flex items-center gap-3 border-b border-[var(--sandbox-console-border)] px-4 py-3 transition-colors last:border-b-0 hover:bg-[var(--sandbox-console-control-hover)]">
                  <StatusIcon status={tx.status} />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-display font-semibold text-[var(--sandbox-console-text)]">{tx.label}</div>
                    <div className="mt-0.5 text-xs font-data text-[var(--sandbox-console-muted)]">
                      <a
                        href={`https://explorer.tangle.tools/tx/${tx.hash}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="transition-colors hover:text-[var(--sandbox-console-brand)]"
                        onClick={(e) => e.stopPropagation()}
                      >
                        {tx.hash.slice(0, 10)}...{tx.hash.slice(-6)}
                      </a>
                      {' · '}{timeAgo(tx.timestamp)}
                    </div>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
