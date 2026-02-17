import { useState, useEffect, useCallback } from 'react';
import { useStore } from '@nanostores/react';
import { useAccount, useWriteContract, useWaitForTransactionReceipt } from 'wagmi';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '~/components/ui/dialog';
import { Button } from '~/components/ui/button';
import { Input } from '~/components/ui/input';
import { Badge } from '~/components/ui/badge';
import { Card, CardContent } from '~/components/ui/card';
import { Identicon } from '~/components/shared/Identicon';
import { infraStore, updateInfra } from '~/lib/stores/infra';
import { useServiceValidation } from '~/lib/hooks/useServiceValidation';
import { useOperators, type DiscoveredOperator } from '~/lib/hooks/useOperators';
import { useQuotes, formatCost } from '~/lib/hooks/useQuotes';
import { tangleServicesAbi } from '~/lib/contracts/abi';
import { getAddresses } from '~/lib/contracts/publicClient';
import { cn } from '~/lib/utils';
import type { Address } from 'viem';

interface InfrastructureModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

type ServiceMode = 'existing' | 'new';

export function InfrastructureModal({ open, onOpenChange }: InfrastructureModalProps) {
  const { address } = useAccount();
  const infra = useStore(infraStore);
  const [blueprintId, setBlueprintId] = useState(infra.blueprintId);
  const [serviceId, setServiceId] = useState(infra.serviceId);
  const [mode, setMode] = useState<ServiceMode>('existing');
  const { validate, isValidating, serviceInfo, error: validationError, reset: resetValidation } = useServiceValidation();

  // Operator discovery for "Create New"
  const { operators, isLoading: operatorsLoading, operatorCount } = useOperators(BigInt(blueprintId || '0'));
  const [selectedOperators, setSelectedOperators] = useState<Address[]>([]);
  const [manualAddr, setManualAddr] = useState('');

  // RFQ quotes
  const TTL_BLOCKS = 864000n; // ~30 days at 3s blocks
  const { quotes, isLoading: quotesLoading, isSolvingPow, errors: quoteErrors, totalCost, refetch: refetchQuotes } = useQuotes(
    operators.filter((op) => selectedOperators.includes(op.address)),
    BigInt(blueprintId || '0'),
    TTL_BLOCKS,
    mode === 'new' && selectedOperators.length > 0,
  );

  // Service creation TX
  const { writeContractAsync, data: createTxHash, isPending: isCreating } = useWriteContract();
  const { isSuccess: createConfirmed } = useWaitForTransactionReceipt({ hash: createTxHash });

  // Handle Verify
  const handleVerify = useCallback(async () => {
    if (!serviceId) return;
    const info = await validate(BigInt(serviceId), address);
    if (info?.active) {
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
        },
      });
    }
  }, [serviceId, blueprintId, address, validate]);

  // Handle Create Service from quotes
  const handleCreateFromQuotes = useCallback(async () => {
    if (quotes.length === 0 || !address) return;
    const addrs = getAddresses();

    const quoteTuples = quotes.map((q) => ({
      details: {
        blueprintId: q.details.blueprintId,
        ttlBlocks: q.details.ttlBlocks,
        totalCost: q.details.totalCost,
        timestamp: q.details.timestamp,
        expiry: q.details.expiry,
        securityCommitments: q.details.securityCommitments.map((sc) => ({
          asset: sc.asset,
          exposureBps: sc.exposureBps,
        })),
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
    setSelectedOperators((prev) =>
      prev.includes(addr) ? prev.filter((a) => a !== addr) : [...prev, addr],
    );
  };

  const addManualOperator = () => {
    if (/^0x[a-fA-F0-9]{40}$/.test(manualAddr) && !selectedOperators.includes(manualAddr as Address)) {
      setSelectedOperators((prev) => [...prev, manualAddr as Address]);
      setManualAddr('');
    }
  };

  // Auto-verify on open if we have a service ID
  useEffect(() => {
    if (open && serviceId && mode === 'existing' && !serviceInfo) {
      handleVerify();
    }
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="font-display">Infrastructure Settings</DialogTitle>
          <DialogDescription>
            Configure the blueprint and service for sandbox provisioning
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
                onClick={() => { setMode('new'); resetValidation(); }}
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
                            <Identicon address={op} size={20} />
                            <span className="text-xs font-data text-cloud-elements-textSecondary truncate">{op}</span>
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
                          <Identicon address={op.address} size={24} />
                          <span className="text-xs font-data text-cloud-elements-textSecondary truncate flex-1">
                            {op.address.slice(0, 8)}...{op.address.slice(-6)}
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
                            <div className="flex items-center gap-2">
                              <Identicon address={q.operator} size={20} />
                              <span className="text-xs font-data text-cloud-elements-textSecondary">
                                {q.operator.slice(0, 8)}...{q.operator.slice(-6)}
                              </span>
                            </div>
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
                          disabled={isCreating}
                        >
                          {isCreating ? 'Creating...' : `Create Service (${formatCost(totalCost)})`}
                        </Button>
                      ) : (
                        <Button
                          className="flex-1"
                          variant="secondary"
                          onClick={handleRequestService}
                          disabled={isCreating || selectedOperators.length === 0}
                        >
                          {isCreating ? 'Creating...' : 'Request Service (No Quotes)'}
                        </Button>
                      )}
                    </div>

                    {createConfirmed && (
                      <div className="glass-card rounded-lg p-3 border-teal-500/30 mt-3">
                        <div className="flex items-center gap-2">
                          <div className="i-ph:check-circle-fill text-sm text-teal-400" />
                          <p className="text-xs text-cloud-elements-textPrimary">
                            Service creation submitted. Watch for ServiceActivated event to get the new service ID.
                          </p>
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
        <div className="flex items-center gap-2">
          <div className="i-ph:globe text-sm text-cloud-elements-textTertiary" />
          <span className="text-xs text-cloud-elements-textTertiary">Blueprint</span>
          <Badge variant="accent">#{infra.blueprintId}</Badge>
        </div>
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
        </div>
      </div>
      <Button variant="ghost" size="sm" onClick={onOpenModal}>
        Change
      </Button>
    </div>
  );
}
