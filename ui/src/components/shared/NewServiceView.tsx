import type { Address } from 'viem';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';
import { formatCost } from '@tangle-network/blueprint-ui';
import type { DiscoveredOperator, OperatorQuote } from '@tangle-network/blueprint-ui';
import { OperatorIdentity } from '~/components/shared/VisualIdentity';

interface NewServiceViewProps {
  selectedOperators: Address[];
  operatorsLoading: boolean;
  operatorsError: Error | null;
  operatorCount: bigint;
  operators: DiscoveredOperator[];
  toggleOperator: (addr: Address) => void;
  manualAddr: string;
  setManualAddr: (value: string) => void;
  addManualOperator: () => void;
  refetchQuotes: () => void;
  quotesLoading: boolean;
  isSolvingPow: boolean;
  quotes: OperatorQuote[];
  totalCost: bigint;
  quoteErrors: Map<Address, string>;
  handleCreateFromQuotes: () => void;
  isCreating: boolean;
  createPending: boolean;
  handleRequestService: () => void;
  createConfirmed: boolean;
  resolvedServiceId: string | null;
  serviceRequestId: number | null;
  createTxHash: `0x${string}` | undefined;
}

export function NewServiceView({
  selectedOperators,
  operatorsLoading,
  operatorsError,
  operatorCount,
  operators,
  toggleOperator,
  manualAddr,
  setManualAddr,
  addManualOperator,
  refetchQuotes,
  quotesLoading,
  isSolvingPow,
  quotes,
  totalCost,
  quoteErrors,
  handleCreateFromQuotes,
  isCreating,
  createPending,
  handleRequestService,
  createConfirmed,
  resolvedServiceId,
  serviceRequestId,
  createTxHash,
}: NewServiceViewProps) {
  return (
    <div className="space-y-4">
      {/* Operator Grid */}
      <div>
        <label className="block text-xs text-cloud-elements-textTertiary mb-2">
          Select Operators ({selectedOperators.length} selected)
        </label>
        {operatorsLoading ? (
          <p className="text-xs text-cloud-elements-textTertiary animate-pulse">Discovering operators...</p>
        ) : operatorsError ? (
          <div className="space-y-1.5">
            <p className="text-xs text-amber-400">
              {operatorCount > 0n
                ? `Found ${operatorCount.toString()} registered operator${operatorCount === 1n ? '' : 's'} on-chain, but verification failed`
                : 'Operator lookup failed for this blueprint'}
            </p>
            <p className="text-[11px] text-cloud-elements-textTertiary">
              You can still add an operator address manually below.
            </p>
          </div>
        ) : operators.length > 0 ? (
          <div className="grid grid-cols-2 gap-2 max-h-40 overflow-y-auto">
            {operators.map((op) => (
              <button
                key={op.address}
                onClick={() => toggleOperator(op.address)}
                className={cn(
                  'flex items-center gap-2 p-2 rounded-lg border text-left transition-all',
                  selectedOperators.includes(op.address)
                    ? 'border-violet-500/30 bg-violet-500/5'
                    : 'border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 hover:bg-cloud-elements-background-depth-3',
                )}
              >
                <span className="min-w-0 flex-1">
                  <OperatorIdentity address={op.address} detail="registered" compact />
                </span>
                {selectedOperators.includes(op.address) && (
                  <div className="i-ph:check-bold text-xs text-violet-400" />
                )}
              </button>
            ))}
          </div>
        ) : (
          <p className="text-xs text-cloud-elements-textTertiary">No operators found for this blueprint</p>
        )}

        {/* Manual address */}
        <div className="flex gap-2 mt-2">
          <Input
            value={manualAddr}
            onChange={(e) => setManualAddr(e.target.value)}
            placeholder="0x... operator address"
            className="text-xs"
          />
          <Button variant="secondary" size="sm" onClick={addManualOperator} disabled={!/^0x[a-fA-F0-9]{40}$/.test(manualAddr)}>
            Add
          </Button>
        </div>
      </div>

      {/* Quotes */}
      {selectedOperators.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs text-cloud-elements-textTertiary">Operator Quotes</label>
            <Button variant="ghost" size="sm" onClick={refetchQuotes} disabled={quotesLoading}>
              <div className="i-ph:arrow-clockwise text-xs" />
              Refresh
            </Button>
          </div>

          {isSolvingPow && (
            <p className="text-xs text-cloud-elements-textTertiary animate-pulse mb-2">
              Solving PoW challenge...
            </p>
          )}

          {quotes.length > 0 && (
            <div className="space-y-2 mb-3">
              {quotes.map((q) => (
                <div key={q.operator} className="flex items-center justify-between p-2 glass-card rounded-lg">
                  <OperatorIdentity address={q.operator} detail="quote" compact />
                  <span className="text-xs font-data font-semibold text-cloud-elements-textPrimary">
                    {formatCost(q.totalCost)}
                  </span>
                </div>
              ))}
              <div className="flex justify-between text-sm pt-1 border-t border-cloud-elements-dividerColor">
                <span className="text-cloud-elements-textSecondary">Total Cost</span>
                <span className="font-data font-semibold">{formatCost(totalCost)}</span>
              </div>
            </div>
          )}

          {quoteErrors.size > 0 && (
            <div className="space-y-1 mb-3">
              {Array.from(quoteErrors.entries()).map(([addr, err]) => (
                <div key={addr} className="text-xs text-cloud-elements-textTertiary">
                  <span className="font-data">{addr.slice(0, 8)}...{addr.slice(-4)}</span>: {err}
                </div>
              ))}
            </div>
          )}

          {/* Deploy buttons */}
          <div className="flex gap-2">
            {quotes.length > 0 ? (
              <Button
                className="flex-1"
                onClick={handleCreateFromQuotes}
                disabled={isCreating || createPending}
              >
                {isCreating || createPending ? 'Creating...' : `Create Service (${formatCost(totalCost)})`}
              </Button>
            ) : (
              <Button
                className="flex-1"
                variant="secondary"
                onClick={handleRequestService}
                disabled={isCreating || createPending || selectedOperators.length === 0}
              >
                {isCreating || createPending ? 'Creating...' : 'Request Service (No Quotes)'}
              </Button>
            )}
          </div>

          {createConfirmed && (
            <div className="glass-card rounded-lg p-3 border-teal-500/30 mt-3">
              <div className="flex items-center gap-2">
                <div className="i-ph:check-circle-fill text-sm text-teal-400" />
                <div className="min-w-0">
                  <p className="text-xs text-cloud-elements-textPrimary">
                    {resolvedServiceId
                      ? `Service #${resolvedServiceId} is active and selected.`
                      : serviceRequestId != null
                        ? `Service request #${serviceRequestId} submitted. Waiting for activation.`
                        : 'Service creation submitted. Waiting for activation.'}
                  </p>
                  {createTxHash ? (
                    <p className="mt-1 truncate font-data text-[11px] text-cloud-elements-textTertiary">{createTxHash}</p>
                  ) : null}
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
