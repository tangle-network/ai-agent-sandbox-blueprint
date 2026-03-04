import { useStore } from '@nanostores/react';
import { txListStore, pendingCount, clearTxs, type TrackedTx } from '@tangle/blueprint-ui';
import { timeAgo, useDropdownMenu } from '@tangle-network/agent-ui/primitives';

function StatusIcon({ status }: { status: TrackedTx['status'] }) {
  if (status === 'pending') return <div className="w-4 h-4 rounded-full border-2 border-violet-500/40 border-t-violet-400 animate-spin shrink-0" />;
  if (status === 'confirmed') return <div className="i-ph:check-circle-fill text-base text-cloud-elements-icon-success shrink-0" />;
  return <div className="i-ph:x-circle-fill text-base text-cloud-elements-icon-error shrink-0" />;
}

export function TxDropdown() {
  const { open, ref, toggle, close } = useDropdownMenu({ closeOnEsc: true });
  const txs = useStore(txListStore);
  const pending = useStore(pendingCount);

  return (
    <div ref={ref} className="relative">
      <button type="button" onClick={toggle} className="relative p-2.5 rounded-lg glass-card hover:border-violet-500/20 transition-all" title="Transaction history">
        <div className="i-ph:receipt text-base text-cloud-elements-textSecondary" />
        {pending > 0 && (
          <span className="absolute -top-1 -right-1 min-w-[18px] h-[18px] flex items-center justify-center px-1 rounded-full bg-violet-600 text-white text-[10px] font-data font-bold animate-pulse">{pending}</span>
        )}
      </button>

      {open && (
        <div className="absolute right-0 top-full mt-2 w-96 rounded-xl border border-cloud-elements-dividerColor/50 z-50 shadow-xl overflow-hidden bg-[var(--cloud-elements-bg-depth-2)]">
          <div className="flex items-center justify-between px-4 py-3 border-b border-cloud-elements-dividerColor/50">
            <div className="flex items-center gap-2">
              <div className="i-ph:clock-counter-clockwise text-base text-cloud-elements-textTertiary" />
              <span className="text-sm font-display font-semibold text-cloud-elements-textPrimary">Transactions</span>
              {txs.length > 0 && <span className="text-xs font-data text-cloud-elements-textTertiary">({txs.length})</span>}
            </div>
            {txs.length > 0 && (
              <button type="button" onClick={() => { clearTxs(); close(); }} className="text-xs font-data text-cloud-elements-textTertiary hover:text-crimson-700 dark:hover:text-crimson-400 transition-colors">Clear all</button>
            )}
          </div>
          <div className="max-h-[400px] overflow-y-auto">
            {txs.length === 0 ? (
              <div className="py-10 text-center">
                <div className="i-ph:receipt text-2xl text-cloud-elements-textTertiary mb-2 mx-auto" />
                <p className="text-sm text-cloud-elements-textTertiary">No transactions yet</p>
              </div>
            ) : (
              txs.map((tx) => (
                <div key={tx.hash} className="flex items-center gap-3 px-4 py-3 border-b border-cloud-elements-dividerColor/50 last:border-b-0 hover:bg-cloud-elements-item-backgroundHover transition-colors">
                  <StatusIcon status={tx.status} />
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-display font-medium text-cloud-elements-textPrimary truncate">{tx.label}</div>
                    <div className="text-xs font-data text-cloud-elements-textTertiary mt-0.5">
                      <a
                        href={`https://explorer.tangle.tools/tx/${tx.hash}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="hover:text-violet-400 transition-colors"
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
