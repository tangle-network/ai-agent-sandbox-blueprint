import { useCallback, useEffect, useMemo, useState } from 'react';
import { useAccount, useWaitForTransactionReceipt, useWriteContract } from 'wagmi';
import type { Address } from 'viem';
import {
  cn,
  formatCost,
  getAddresses,
  tangleServicesAbi,
  updateInfra,
  useQuotes,
  type DiscoveredOperator,
} from '@tangle-network/blueprint-ui';
import { OperatorIdentity } from '~/components/shared/VisualIdentity';
import { truncateAddress } from '~/lib/utils/truncate-address';
import { LaunchActionButton, LaunchInput, launchControlClass } from './launch-fields';
import type { DeployStepProps } from './deploy-support';
import {
  SERVICE_TTL_BLOCKS,
  ZERO_REQUESTER,
  getRequestIdFromServiceReceiptLogs,
  isValidAddress,
  parsePositiveServiceId,
  resolveActivatedServiceId,
  type ServiceReceiptLog,
} from './support';

export function ServiceSetupPanel({
  blueprintId,
  currentServiceId,
  operators,
  operatorsLoading,
  operatorsError,
  operatorCount,
  validateService,
}: {
  blueprintId: string;
  currentServiceId: string;
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  operatorsError?: Error | null;
  operatorCount: bigint;
  validateService: DeployStepProps['validateService'];
}) {
  const { address } = useAccount();
  const [mode, setMode] = useState<'new' | 'existing'>('new');
  const [selectedOperators, setSelectedOperators] = useState<Address[]>([]);
  const [manualOperator, setManualOperator] = useState('');
  const [serviceIdInput, setServiceIdInput] = useState(currentServiceId || '');
  const [serviceCheckMessage, setServiceCheckMessage] = useState<string | null>(null);
  const [serviceCheckError, setServiceCheckError] = useState<string | null>(null);
  const [serviceCreateError, setServiceCreateError] = useState<string | null>(null);
  const [resolvedServiceId, setResolvedServiceId] = useState<string | null>(null);

  useEffect(() => {
    setServiceIdInput(currentServiceId || '');
  }, [currentServiceId]);

  useEffect(() => {
    if (selectedOperators.length > 0 || operators.length === 0) return;
    setSelectedOperators([operators[0].address as Address]);
  }, [operators, selectedOperators.length]);

  const selectedOperatorRecords = useMemo(
    (): DiscoveredOperator[] => selectedOperators.map((operator) => {
      const discovered = operators.find((item) => item.address.toLowerCase() === operator.toLowerCase());
      return discovered ?? { address: operator, ecdsaPublicKey: '0x', rpcAddress: '' };
    }),
    [operators, selectedOperators],
  );
  const manualOperatorRecords = selectedOperatorRecords.filter((operator) =>
    !operators.some((item) => item.address.toLowerCase() === operator.address.toLowerCase()),
  );
  const serviceIdToCheck = parsePositiveServiceId(serviceIdInput);

  const { quotes, isLoading: quotesLoading, isSolvingPow, errors: quoteErrors, totalCost, refetch: refetchQuotes } = useQuotes(
    selectedOperatorRecords,
    BigInt(blueprintId || '0'),
    SERVICE_TTL_BLOCKS,
    mode === 'new' && selectedOperators.length > 0 && !!address,
    (address ?? ZERO_REQUESTER) as `0x${string}`,
  );

  const { writeContractAsync, data: serviceTxHash, isPending: serviceSigning } = useWriteContract();
  const {
    data: serviceReceipt,
    isSuccess: serviceConfirmed,
    isLoading: servicePending,
  } = useWaitForTransactionReceipt({ hash: serviceTxHash });

  const serviceRequestId = useMemo(() => {
    if (!serviceReceipt?.logs) return null;
    return getRequestIdFromServiceReceiptLogs(serviceReceipt.logs as ServiceReceiptLog[]);
  }, [serviceReceipt]);

  const updateServiceFromValidation = useCallback((nextServiceId: string, info: Awaited<ReturnType<DeployStepProps['validateService']>>) => {
    if (!info) {
      updateInfra({ blueprintId, serviceId: nextServiceId, serviceValidated: false, serviceInfo: undefined });
      return;
    }

    updateInfra({
      blueprintId,
      serviceId: nextServiceId,
      serviceValidated: info.active && info.permitted,
      serviceInfo: {
        active: info.active,
        operatorCount: info.operatorCount,
        owner: info.owner,
        blueprintId: String(info.blueprintId),
        permitted: info.permitted,
        operators: (info.operators ?? []).map((operator) => {
          const discovered = operators.find((item) => item.address.toLowerCase() === operator.toLowerCase());
          return { address: operator, rpcAddress: discovered?.rpcAddress ?? '' };
        }),
      },
    });
  }, [blueprintId, operators]);

  const validateAndSelectService = useCallback(async (nextServiceId: string) => {
    const parsedServiceId = parsePositiveServiceId(nextServiceId);
    setServiceCheckMessage(null);
    setServiceCheckError(null);

    if (!parsedServiceId) {
      setServiceCheckError('Enter a positive whole-number service ID.');
      return;
    }

    const normalizedServiceId = parsedServiceId.toString();
    updateInfra({ blueprintId, serviceId: normalizedServiceId, serviceValidated: false });

    try {
      const info = await validateService(parsedServiceId, address as `0x${string}` | undefined);
      updateServiceFromValidation(normalizedServiceId, info);
      if (!info) {
        setServiceCheckError(`Service #${normalizedServiceId} was not found.`);
      } else if (!info.active) {
        setServiceCheckError(`Service #${normalizedServiceId} exists but is inactive.`);
      } else if (!info.permitted) {
        setServiceCheckError(`This wallet is not permitted on service #${normalizedServiceId}.`);
      } else {
        setServiceCheckMessage(`Service #${normalizedServiceId} is active and selected.`);
      }
    } catch (error) {
      setServiceCheckError(error instanceof Error ? error.message : `Service #${normalizedServiceId} could not be checked.`);
    }
  }, [address, blueprintId, updateServiceFromValidation, validateService]);

  const commitResolvedService = useCallback(async (nextServiceId: string) => {
    setResolvedServiceId(nextServiceId);
    setServiceIdInput(nextServiceId);
    await validateAndSelectService(nextServiceId);
  }, [validateAndSelectService]);

  useEffect(() => {
    if (!serviceConfirmed || serviceRequestId == null || resolvedServiceId) return;

    let cancelled = false;
    const tick = async () => {
      try {
        const nextServiceId = await resolveActivatedServiceId(serviceRequestId);
        if (!cancelled && nextServiceId) {
          await commitResolvedService(nextServiceId);
        }
      } catch {
        // Operator approval may not have activated the service yet.
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
  }, [commitResolvedService, resolvedServiceId, serviceConfirmed, serviceRequestId]);

  const toggleOperator = (operator: Address) => {
    setSelectedOperators((current) =>
      current.some((item) => item.toLowerCase() === operator.toLowerCase())
        ? current.filter((item) => item.toLowerCase() !== operator.toLowerCase())
        : [...current, operator],
    );
  };

  const removeOperator = (operator: Address) => {
    setSelectedOperators((current) => current.filter((item) => item.toLowerCase() !== operator.toLowerCase()));
  };

  const addManualOperator = () => {
    const next = manualOperator.trim() as Address;
    if (!isValidAddress(next)) return;
    setSelectedOperators((current) =>
      current.some((item) => item.toLowerCase() === next.toLowerCase()) ? current : [...current, next],
    );
    setManualOperator('');
  };

  const createService = async () => {
    if (!address || selectedOperators.length === 0) return;
    setServiceCreateError(null);
    setResolvedServiceId(null);
    const addrs = getAddresses();

    try {
      if (quotes.length > 0) {
        const quoteTuples = quotes.map((quote) => ({
          details: {
            requester: quote.details.requester,
            blueprintId: quote.details.blueprintId,
            ttlBlocks: quote.details.ttlBlocks,
            totalCost: quote.details.totalCost,
            timestamp: quote.details.timestamp,
            expiry: quote.details.expiry,
            confidentiality: quote.details.confidentiality,
            securityCommitments: quote.details.securityCommitments.map((commitment) => ({
              asset: commitment.asset,
              exposureBps: commitment.exposureBps,
            })),
            resourceCommitments: quote.details.resourceCommitments,
          },
          signature: quote.signature,
          operator: quote.operator,
        }));

        await writeContractAsync({
          address: addrs.services,
          abi: tangleServicesAbi,
          functionName: 'createServiceFromQuotes',
          args: [
            BigInt(blueprintId),
            quoteTuples,
            '0x' as `0x${string}`,
            [address],
            SERVICE_TTL_BLOCKS,
          ],
          value: totalCost,
        });
        return;
      }

      await writeContractAsync({
        address: addrs.services,
        abi: tangleServicesAbi,
        functionName: 'requestService',
        args: [
          BigInt(blueprintId),
          selectedOperators,
          '0x' as `0x${string}`,
          [address],
          SERVICE_TTL_BLOCKS,
          '0x0000000000000000000000000000000000000000' as Address,
          0n,
        ],
      });
    } catch (error) {
      setServiceCreateError(error instanceof Error ? error.message : 'Service request failed.');
    }
  };

  const busy = serviceSigning || servicePending;
  const canCreate = !!address && selectedOperators.length > 0 && !busy;
  const quoteSummary = quotesLoading
    ? 'Loading operator quote'
    : quotes.length > 0
      ? `Quote ready: ${formatCost(totalCost)}`
      : selectedOperators.length > 0
        ? 'No quote yet; service request can still be submitted'
        : 'Select at least one operator';

  return (
    <div className="space-y-3 rounded-[4px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-panel)] p-3.5">
      <div className="grid grid-cols-2 gap-1 rounded-[4px] bg-[var(--sandbox-console-control)] p-1">
        {([
          ['new', 'New service'],
          ['existing', 'Existing service'],
        ] as const).map(([value, label]) => (
          <button
            key={value}
            type="button"
            onClick={() => {
              setMode(value);
              setServiceCheckMessage(null);
              setServiceCheckError(null);
              setServiceCreateError(null);
            }}
            className={cn(
              'h-9 rounded-[3px] font-display text-xs font-bold transition-[background-color,box-shadow,color]',
              mode === value
                ? 'bg-[var(--sandbox-console-brand-soft)] text-[var(--sandbox-console-text)] shadow-[inset_0_0_0_1px_var(--sandbox-console-brand-border)]'
                : 'text-[var(--sandbox-console-muted)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)]',
            )}
            aria-pressed={mode === value}
          >
            {label}
          </button>
        ))}
      </div>

      {mode === 'new' ? (
        <div className="space-y-3">
          <div>
            <div className="flex items-center justify-between gap-3">
              <p className="font-data text-[11px] font-bold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">Operator</p>
              {operatorCount > 0n ? (
                <span className="font-data text-xs font-semibold text-[var(--sandbox-console-subtle)]">{operatorCount.toString()} registered</span>
              ) : null}
            </div>
            <div className="mt-2 max-h-36 space-y-1.5 overflow-y-auto">
              {operatorsLoading ? (
                <div className="flex items-center gap-2 text-sm text-[var(--sandbox-console-muted)]">
                  <span className="h-3 w-3 animate-spin rounded-full border border-[var(--sandbox-console-muted)] border-t-transparent" />
                  Loading operators
                </div>
              ) : operators.length > 0 ? (
                operators.map((operator) => {
                  const selected = selectedOperators.some((item) => item.toLowerCase() === operator.address.toLowerCase());
                  return (
                    <button
                      key={operator.address}
                      type="button"
                      onClick={() => toggleOperator(operator.address as Address)}
                      className={cn(
                        'flex w-full items-center justify-between gap-2 rounded-[4px] border px-2.5 py-2 text-left transition-[background-color,border-color,box-shadow]',
                        selected
                          ? 'border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] shadow-[inset_3px_0_0_var(--sandbox-console-brand)]'
                          : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)]',
                      )}
                    >
                      <OperatorIdentity address={operator.address} detail={operator.rpcAddress || 'registered operator'} compact />
                      {selected ? <span className="i-ph:check-bold shrink-0 text-xs text-[var(--sandbox-console-brand)]" /> : null}
                    </button>
                  );
                })
              ) : (
                <p className="text-sm leading-6 text-[var(--sandbox-console-muted)]">
                  {operatorsError
                    ? 'Operator lookup failed. Add an operator address manually.'
                    : 'No operators were discovered for this blueprint.'}
                </p>
              )}
            </div>
          </div>

          <div className="flex gap-2">
            <input
              aria-label="Manual operator address"
              value={manualOperator}
              onChange={(event) => setManualOperator(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && isValidAddress(manualOperator)) {
                  event.preventDefault();
                  addManualOperator();
                }
              }}
              placeholder="0x... operator"
              className={cn(launchControlClass, 'min-h-9 py-2 text-xs')}
            />
            <LaunchActionButton
              variant="secondary"
              size="sm"
              onClick={addManualOperator}
              disabled={!isValidAddress(manualOperator)}
            >
              Add
            </LaunchActionButton>
          </div>

          {manualOperatorRecords.length > 0 ? (
            <div className="space-y-1.5">
              <p className="font-data text-[11px] font-bold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]">
                Added manually
              </p>
              {manualOperatorRecords.map((operator) => (
                <div
                  key={operator.address}
                  className="flex items-center justify-between gap-2 rounded-[4px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-2.5 py-2"
                >
                  <OperatorIdentity address={operator.address} detail="manual operator" compact />
                  <button
                    type="button"
                    onClick={() => removeOperator(operator.address)}
                    className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[3px] text-[var(--sandbox-console-muted)] transition-colors hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-danger)]"
                    aria-label={`Remove operator ${truncateAddress(operator.address)}`}
                  >
                    <span className="i-ph:x text-sm" />
                  </button>
                </div>
              ))}
            </div>
          ) : null}

          <div className="rounded-[4px] bg-[var(--sandbox-console-control)] px-3 py-2">
            <div className="flex items-center justify-between gap-2">
              <span className="truncate text-xs font-semibold text-[var(--sandbox-console-secondary)]">
                {isSolvingPow ? 'Solving quote challenge' : quoteSummary}
              </span>
              <button
                type="button"
                onClick={() => refetchQuotes()}
                disabled={quotesLoading || selectedOperators.length === 0 || !address}
                className="inline-flex h-7 items-center gap-1 rounded-[3px] px-2 font-display text-xs font-bold text-[var(--sandbox-console-muted)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] disabled:cursor-not-allowed disabled:opacity-50"
              >
                <span className="i-ph:arrow-clockwise text-xs" />
                Refresh
              </button>
            </div>
            {quoteErrors.size > 0 ? (
              <p className="mt-1 text-xs text-amber-400">{quoteErrors.size} quote request{quoteErrors.size === 1 ? '' : 's'} failed.</p>
            ) : null}
          </div>

          <LaunchActionButton size="lg" className="w-full" onClick={createService} disabled={!canCreate}>
            {busy ? (
              <>
                <span className="h-4 w-4 animate-spin rounded-full border-2 border-white/40 border-t-white" />
                Creating service
              </>
            ) : (
              <>
                <span className="i-ph:plus-circle text-base" />
                Create service
              </>
            )}
          </LaunchActionButton>

          {!address ? <p className="text-xs text-[var(--sandbox-console-danger)]">Connect a wallet before creating a service.</p> : null}
          {serviceCreateError ? <p className="text-xs text-[var(--sandbox-console-danger)]">{serviceCreateError}</p> : null}
          {serviceConfirmed ? (
            <div className="rounded-[4px] border border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] px-3 py-2 text-sm text-[var(--sandbox-console-success)]">
              {resolvedServiceId
                ? `Service #${resolvedServiceId} is active and selected.`
                : serviceRequestId != null
                  ? `Service request #${serviceRequestId} submitted. Waiting for activation.`
                  : 'Service request submitted. Waiting for activation.'}
            </div>
          ) : null}
        </div>
      ) : (
        <div className="space-y-3">
          <LaunchInput
            label="Service ID"
            type="number"
            min={1}
            value={serviceIdInput}
            onChange={(event) => setServiceIdInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && serviceIdToCheck) {
                event.preventDefault();
                void validateAndSelectService(serviceIdInput);
              }
            }}
            placeholder="1"
          />
          <LaunchActionButton
            size="lg"
            className="w-full"
            onClick={() => validateAndSelectService(serviceIdInput)}
            disabled={!serviceIdToCheck}
          >
            <span className="i-ph:magnifying-glass text-base" />
            Check service
          </LaunchActionButton>
          {serviceCheckMessage ? <p className="text-sm text-[var(--sandbox-console-success)]">{serviceCheckMessage}</p> : null}
          {serviceCheckError ? <p className="text-sm text-[var(--sandbox-console-danger)]">{serviceCheckError}</p> : null}
        </div>
      )}
    </div>
  );
}
