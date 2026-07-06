import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { useAccount, useWriteContract, useWaitForTransactionReceipt } from 'wagmi';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { infraStore, updateInfra } from '@tangle-network/blueprint-ui';
import { useServiceValidation } from '@tangle-network/blueprint-ui';
import { useQuotes } from '@tangle-network/blueprint-ui';
import { tangleServicesAbi } from '@tangle-network/blueprint-ui';
import { getAddresses } from '@tangle-network/blueprint-ui';
import { cn } from '@tangle-network/blueprint-ui';
import { useReliableOperators } from '~/lib/hooks/useReliableOperators';
import type { Address } from 'viem';
import { ExistingServiceView } from './ExistingServiceView';
import { NewServiceView } from './NewServiceView';
import {
  getRequestIdFromServiceReceiptLogs,
  resolveActivatedServiceId,
  type ServiceReceiptLog,
} from './serviceResolution';

export { InfraBar } from './InfraBar';

interface InfrastructureModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialMode?: ServiceMode;
}

type ServiceMode = 'existing' | 'new';

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
              <ExistingServiceView
                serviceId={serviceId}
                setServiceId={setServiceId}
                resetValidation={resetValidation}
                handleVerify={handleVerify}
                isValidating={isValidating}
                validationError={validationError}
                serviceInfo={serviceInfo}
              />
            )}

            {/* Create New Service */}
            {mode === 'new' && (
              <NewServiceView
                selectedOperators={selectedOperators}
                operatorsLoading={operatorsLoading}
                operatorsError={operatorsError}
                operatorCount={operatorCount}
                operators={operators}
                toggleOperator={toggleOperator}
                manualAddr={manualAddr}
                setManualAddr={setManualAddr}
                addManualOperator={addManualOperator}
                refetchQuotes={refetchQuotes}
                quotesLoading={quotesLoading}
                isSolvingPow={isSolvingPow}
                quotes={quotes}
                totalCost={totalCost}
                quoteErrors={quoteErrors}
                handleCreateFromQuotes={handleCreateFromQuotes}
                isCreating={isCreating}
                createPending={createPending}
                handleRequestService={handleRequestService}
                createConfirmed={createConfirmed}
                resolvedServiceId={resolvedServiceId}
                serviceRequestId={serviceRequestId}
                createTxHash={createTxHash}
              />
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
