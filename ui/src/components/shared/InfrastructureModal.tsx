import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { useAccount, useWriteContract, useWaitForTransactionReceipt } from 'wagmi';
import { decodeEventLog } from 'viem';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { infraStore, updateInfra } from '@tangle-network/blueprint-ui';
import { useServiceValidation } from '@tangle-network/blueprint-ui';
import type { DiscoveredOperator } from '@tangle-network/blueprint-ui';
import { useQuotes, formatCost } from '@tangle-network/blueprint-ui';
import { tangleServicesAbi } from '@tangle-network/blueprint-ui';
import { getAddresses, publicClient } from '@tangle-network/blueprint-ui';
import { cn } from '@tangle-network/blueprint-ui';
import { BlueprintBadgeInline } from './InfraSummaryBits';
import { extractServiceRequestId } from '~/lib/contracts/serviceEvents';
import { OperatorIdentity } from '~/components/shared/VisualIdentity';
import { useReliableOperators } from '~/lib/hooks/useReliableOperators';
import type { Address } from 'viem';

interface InfrastructureModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialMode?: ServiceMode;
}

type ServiceMode = 'existing' | 'new';

type ServiceReceiptLog = {
  data: `0x${string}`;
  topics: readonly `0x${string}`[];
};

function getRequestIdFromServiceReceiptLogs(logs: ServiceReceiptLog[]): number | null {
  for (const log of logs) {
    const requestId = extractServiceRequestId(log);
    if (requestId != null) return requestId;
  }

  return null;
}

async function resolveActivatedServiceId(requestId: number): Promise<string | null> {
  const addrs = getAddresses();
  const logs = await publicClient.getLogs({
    address: addrs.services,
    fromBlock: 0n,
    toBlock: 'latest',
  });

  for (const log of logs) {
    try {
      const decoded = decodeEventLog({
        abi: tangleServicesAbi,
        data: log.data,
        topics: [...log.topics] as [] | [`0x${string}`, ...`0x${string}`[]],
      });
      if (decoded.eventName !== 'ServiceActivated') continue;
      if (!('requestId' in decoded.args) || !('serviceId' in decoded.args)) continue;
      if (Number(decoded.args.requestId) !== requestId) continue;
      return String(decoded.args.serviceId);
    } catch {
      // Ignore unrelated logs while scanning the service manager.
    }
  }

  return null;
}

export function InfrastructureModal({ open, onOpenChange, initialMode = 'existing' }: InfrastructureModalProps) {
  const { address } = useAccount();
  const infra = useStore(infraStore);
  const [blueprintId, setBlueprintId] = useState(infra.blueprintId);
  const [serviceId, setServiceId] = useState(infra.serviceId);
  const [mode, setMode] = useState<ServiceMode>(initialMode);
  const { validate, isValidating, serviceInfo, error: validationError, reset: resetValidation } = useServiceValidation();

  // Operator discovery for "Create New"
  const {
    operators,
    isLoading: operatorsLoading,
    operatorCount,
    error: operatorsError,
  } = useReliableOperators(blueprintId || '0');
  const [selectedOperators, setSelectedOperators] = useState<Address[]>([]);
  const [operatorSelectionTouched, setOperatorSelectionTouched] = useState(false);
  const [manualAddr, setManualAddr] = useState('');
  const [resolvedServiceId, setResolvedServiceId] = useState<string | null>(null);
  const wasOpenRef = useRef(false);

  // RFQ quotes — memoize filtered operators to prevent useQuotes from re-running every render
  const TTL_BLOCKS = 864000n; // ~30 days at 3s blocks
  const selectedOps = useMemo(
    () => operators.filter((op) => selectedOperators.includes(op.address)),
    [operators, selectedOperators],
  );
  // tnt-core v0.13.0 binds quotes to a `requester` so per-account pricing
  // (rate limits, holder discounts) can scope to the actual caller. Pass the
  // connected wallet; gate `enabled` on `!!address` so we don't query with
  // the zero-address sentinel.
  const ZERO_REQUESTER = '0x0000000000000000000000000000000000000000' as const;
  const { quotes, isLoading: quotesLoading, isSolvingPow, errors: quoteErrors, totalCost, refetch: refetchQuotes } = useQuotes(
    selectedOps,
    BigInt(blueprintId || '0'),
    TTL_BLOCKS,
    mode === 'new' && selectedOperators.length > 0 && !!address,
    (address ?? ZERO_REQUESTER) as `0x${string}`,
  );

  // Service creation TX
  const { writeContractAsync, data: createTxHash, isPending: isCreating } = useWriteContract();
  const {
    data: createReceipt,
    isSuccess: createConfirmed,
    isLoading: createPending,
  } = useWaitForTransactionReceipt({ hash: createTxHash });

  const serviceRequestId = useMemo(() => {
    if (!createReceipt?.logs) return null;
    return getRequestIdFromServiceReceiptLogs(createReceipt.logs as ServiceReceiptLog[]);
  }, [createReceipt]);

  useEffect(() => {
    const justOpened = open && !wasOpenRef.current;
    wasOpenRef.current = open;
    if (!justOpened) return;

    setBlueprintId(infra.blueprintId);
    setServiceId(infra.serviceId);
    setMode(initialMode);
    setSelectedOperators([]);
    setOperatorSelectionTouched(false);
    setManualAddr('');
    setResolvedServiceId(null);
    resetValidation();
  }, [open, infra.blueprintId, infra.serviceId, initialMode, resetValidation]);

  useEffect(() => {
    if (mode !== 'new') return;
    if (operatorSelectionTouched || selectedOperators.length > 0 || operators.length === 0) return;
    setSelectedOperators([operators[0].address]);
  }, [mode, operatorSelectionTouched, operators, selectedOperators.length]);

  // Handle Verify
  const handleVerify = useCallback(async () => {
    if (!serviceId) return;
    const info = await validate(BigInt(serviceId), address);
    if (info?.active) {
      // Build operator info with RPC addresses from the discovered operators
      const operatorInfos = info.operators
        .map((addr) => {
          const discovered = operators.find((op) => op.address === addr);
          return { address: addr, rpcAddress: discovered?.rpcAddress ?? '' };
        })
        .filter((op) => op.rpcAddress);

      updateInfra({
        serviceId,
        blueprintId,
        serviceValidated: true,
        serviceInfo: {
          active: info.active,
          operatorCount: info.operatorCount,
          owner: info.owner,
          blueprintId: String(info.blueprintId),
          permitted: info.permitted,
          operators: operatorInfos,
        },
      });
    }
  }, [serviceId, blueprintId, address, validate, operators]);

  const commitResolvedService = useCallback(async (nextServiceId: string) => {
    setResolvedServiceId(nextServiceId);
    setServiceId(nextServiceId);

    try {
      const info = await validate(BigInt(nextServiceId), address);
      if (info?.active) {
        const operatorInfos = info.operators
          .map((addr) => {
            const discovered = operators.find((op) => op.address === addr);
            return { address: addr, rpcAddress: discovered?.rpcAddress ?? '' };
          })
          .filter((op) => op.rpcAddress);

        updateInfra({
          serviceId: nextServiceId,
          blueprintId,
          serviceValidated: true,
          serviceInfo: {
            active: info.active,
            operatorCount: info.operatorCount,
            owner: info.owner,
            blueprintId: String(info.blueprintId),
            permitted: info.permitted,
            operators: operatorInfos,
          },
        });
        return;
      }
    } catch {
      // Keep the resolved ID selected even if validation needs a later retry.
    }

    updateInfra({
      serviceId: nextServiceId,
      blueprintId,
      serviceValidated: false,
    });
  }, [address, blueprintId, operators, validate]);

  useEffect(() => {
    if (!open || !createConfirmed || serviceRequestId == null || resolvedServiceId) return;

    let cancelled = false;
    const tick = async () => {
      try {
        const nextServiceId = await resolveActivatedServiceId(serviceRequestId);
        if (!cancelled && nextServiceId) {
          await commitResolvedService(nextServiceId);
        }
      } catch {
        // Service requests may need operator approval before activation.
      }
    };

    void tick();
    const intervalId = window.setInterval(() => {
      void tick();
    }, 5_000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [commitResolvedService, createConfirmed, open, resolvedServiceId, serviceRequestId]);

  // Handle Create Service from quotes
  const handleCreateFromQuotes = useCallback(async () => {
    if (quotes.length === 0 || !address) return;
    const addrs = getAddresses();

    // tnt-core v0.13.0 QuoteDetails shape adds three fields the operator
    // signs over: `requester`, `confidentiality`, `resourceCommitments`.
    // Forward them verbatim from the quote response — the operator's
    // signature covers them, so any local rewrite would invalidate the
    // quote.
    const quoteTuples = quotes.map((q) => ({
      details: {
        requester: q.details.requester,
        blueprintId: q.details.blueprintId,
        ttlBlocks: q.details.ttlBlocks,
        totalCost: q.details.totalCost,
        timestamp: q.details.timestamp,
        expiry: q.details.expiry,
        confidentiality: q.details.confidentiality,
        securityCommitments: q.details.securityCommitments.map((sc) => ({
          asset: sc.asset,
          exposureBps: sc.exposureBps,
        })),
        resourceCommitments: q.details.resourceCommitments,
      },
      signature: q.signature,
      operator: q.operator,
    }));

    try {
      await writeContractAsync({
        address: addrs.services,
        abi: tangleServicesAbi,
        functionName: 'createServiceFromQuotes',
        args: [
          BigInt(blueprintId),
          quoteTuples,
          '0x' as `0x${string}`,
          [address],
          TTL_BLOCKS,
        ],
        value: totalCost,
      });
    } catch {
      // handled by wagmi
    }
  }, [quotes, address, blueprintId, totalCost, writeContractAsync]);

  // Handle Create Service without quotes (requestService)
  const handleRequestService = useCallback(async () => {
    if (selectedOperators.length === 0 || !address) return;
    const addrs = getAddresses();

    try {
      await writeContractAsync({
        address: addrs.services,
        abi: tangleServicesAbi,
        functionName: 'requestService',
        args: [
          BigInt(blueprintId),
          selectedOperators,
          '0x' as `0x${string}`,
          [address],
          TTL_BLOCKS,
          '0x0000000000000000000000000000000000000000' as Address, // native token
          0n,
        ],
      });
    } catch {
      // handled by wagmi
    }
  }, [selectedOperators, address, blueprintId, writeContractAsync]);

  // Toggle operator selection
  const toggleOperator = (addr: Address) => {
    setOperatorSelectionTouched(true);
    setSelectedOperators((prev) =>
      prev.includes(addr) ? prev.filter((a) => a !== addr) : [...prev, addr],
    );
  };

  const addManualOperator = () => {
    if (/^0x[a-fA-F0-9]{40}$/.test(manualAddr) && !selectedOperators.includes(manualAddr as Address)) {
      setOperatorSelectionTouched(true);
      setSelectedOperators((prev) => [...prev, manualAddr as Address]);
      setManualAddr('');
    }
  };

  // Auto-verify on open if we have a service ID (ref guard prevents infinite loop)
  const hasAutoVerified = useRef(false);
  useEffect(() => {
    if (open && serviceId && mode === 'existing' && !serviceInfo && !hasAutoVerified.current) {
      hasAutoVerified.current = true;
      handleVerify();
    }
    if (!open) {
      hasAutoVerified.current = false;
    }
  }, [open, serviceId, mode, serviceInfo, handleVerify]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="font-display">Infrastructure Settings</DialogTitle>
          <DialogDescription>
            Select an active service or create one from registered operators.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-6 mt-2">
          {/* Blueprint ID */}
          <div>
            <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">
              Blueprint ID
            </label>
            <div className="flex gap-2 items-center">
              <Input
                type="number"
                value={blueprintId}
                onChange={(e) => {
                  setBlueprintId(e.target.value);
                  updateInfra({ blueprintId: e.target.value, serviceValidated: false });
                  resetValidation();
                }}
                min={0}
              />
              {operatorCount > 0n && (
                <Badge variant="running">{String(operatorCount)} operators</Badge>
              )}
            </div>
          </div>

          {/* Service Mode Toggle */}
          <div>
            <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">
              Service
            </label>
            <div className="flex gap-2 mb-4">
              <button
                onClick={() => { setMode('existing'); resetValidation(); }}
                className={cn(
                  'flex-1 py-2 px-3 rounded-lg text-sm font-display font-medium transition-all border',
                  mode === 'existing'
                    ? 'bg-violet-500/10 text-violet-700 dark:text-violet-400 border-violet-500/20'
                    : 'bg-cloud-elements-background-depth-2 text-cloud-elements-textSecondary border-cloud-elements-borderColor hover:bg-cloud-elements-background-depth-3',
                )}
              >
                Use Existing
              </button>
              <button
                onClick={() => {
                  setMode('new');
                  setOperatorSelectionTouched(false);
                  resetValidation();
                }}
                className={cn(
                  'flex-1 py-2 px-3 rounded-lg text-sm font-display font-medium transition-all border',
                  mode === 'new'
                    ? 'bg-violet-500/10 text-violet-700 dark:text-violet-400 border-violet-500/20'
                    : 'bg-cloud-elements-background-depth-2 text-cloud-elements-textSecondary border-cloud-elements-borderColor hover:bg-cloud-elements-background-depth-3',
                )}
              >
                Create New
              </button>
            </div>

            {/* Existing Service */}
            {mode === 'existing' && (
              <div className="space-y-4">
                <div className="flex gap-2">
                  <Input
                    type="number"
                    value={serviceId}
                    onChange={(e) => {
                      setServiceId(e.target.value);
                      resetValidation();
                    }}
                    placeholder="Service ID"
                    min={0}
                  />
                  <Button
                    variant="secondary"
                    onClick={handleVerify}
                    disabled={isValidating || !serviceId}
                  >
                    {isValidating ? 'Checking...' : 'Verify'}
                  </Button>
                </div>

                {validationError && (
                  <div className="glass-card rounded-lg p-3 border-crimson-500/30">
                    <p className="text-xs text-crimson-400">{validationError}</p>
                  </div>
                )}

                {serviceInfo && (
                  <div className="glass-card rounded-lg p-4 space-y-3">
                    <div className="flex items-center gap-2">
                      <div className={cn(
                        'w-2 h-2 rounded-full',
                        serviceInfo.active ? 'bg-teal-400' : 'bg-crimson-400',
                      )} />
                      <span className="text-sm font-display font-medium">
                        Service #{serviceId}
                      </span>
                      <Badge variant={serviceInfo.active ? 'running' : 'destructive'}>
                        {serviceInfo.active ? 'Active' : 'Inactive'}
                      </Badge>
                    </div>

                    <div className="grid grid-cols-2 gap-2 text-xs">
                      <div>
                        <span className="text-cloud-elements-textTertiary">Owner</span>
                        <p className="font-data text-cloud-elements-textPrimary truncate">{serviceInfo.owner}</p>
                      </div>
                      <div>
                        <span className="text-cloud-elements-textTertiary">Operators</span>
                        <p className="font-data text-cloud-elements-textPrimary">{serviceInfo.operatorCount}</p>
                      </div>
                    </div>

                    {serviceInfo.operators.length > 0 && (
                      <div className="space-y-1">
                        <span className="text-xs text-cloud-elements-textTertiary">Operator Addresses</span>
                        {serviceInfo.operators.slice(0, 5).map((op) => (
                          <div key={op} className="flex items-center gap-2">
                            <OperatorIdentity address={op} detail="service member" compact />
                          </div>
                        ))}
                      </div>
                    )}

                    {!serviceInfo.permitted && (
                      <div className="glass-card rounded p-2 border-amber-500/30">
                        <p className="text-xs text-amber-400">
                          Your address is not a permitted caller. You may need to be added.
                        </p>
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}

            {/* Create New Service */}
            {mode === 'new' && (
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
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Compact infrastructure bar shown at the top of the wizard.
 * Shows current blueprint + service, with a "Change" button to open the modal.
 */
export function InfraBar({ onOpenModal }: { onOpenModal: () => void }) {
  const infra = useStore(infraStore);

  return (
    <div className="glass-card rounded-lg p-3 flex items-center justify-between mb-6">
      <div className="flex items-center gap-4">
        <BlueprintBadgeInline blueprintId={infra.blueprintId} />
        <div className="flex items-center gap-2">
          <div className="i-ph:cpu text-sm text-cloud-elements-textTertiary" />
          <span className="text-xs text-cloud-elements-textTertiary">Service</span>
          <Badge variant={infra.serviceValidated ? 'running' : 'secondary'}>
            #{infra.serviceId}
          </Badge>
          {infra.serviceValidated && infra.serviceInfo && (
            <span className="text-xs text-cloud-elements-textTertiary">
              ({infra.serviceInfo.operatorCount} operators)
            </span>
          )}
          {!infra.serviceValidated && (
            <div className="i-ph:warning text-xs text-amber-400" title="Service not validated" />
          )}
        </div>
      </div>
      <Button variant="ghost" size="sm" onClick={onOpenModal}>
        Change
      </Button>
    </div>
  );
}
