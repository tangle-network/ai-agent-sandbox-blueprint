import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { decodeEventLog } from 'viem';
import { getAddresses, publicClient, tangleServicesAbi } from '@tangle-network/blueprint-ui';
import { useOperatorAuth } from './useOperatorAuth';
import { instanceListStore, type LocalInstance } from '~/lib/stores/instances';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { reconcileInstances } from './instanceHydrationLogic';
import { fetchSandboxes, type ApiSandbox } from './sandboxHydrationLogic';
import type { SandboxAddresses } from '~/lib/contracts/chains';
import { extractServiceRequestId } from '~/lib/contracts/serviceEvents';

const DRAFT_TX_GRACE_MS = 15 * 60 * 1000;

interface RefreshOpts {
  interactive?: boolean;
}

export interface InstanceHydrationState {
  refresh: (opts?: RefreshOpts) => Promise<boolean>;
  isHydrating: boolean;
  authRequired: boolean;
  lastError: string | null;
}

function hasRecentPendingTx(instance: LocalInstance): boolean {
  if (!instance.txHash || instance.status !== 'creating') return false;
  return Date.now() - instance.createdAt <= DRAFT_TX_GRACE_MS;
}

function getRequestIdFromReceiptLogs(
  logs: Array<{ data: `0x${string}`; topics: readonly `0x${string}`[] }>,
): number | null {
  for (const log of logs) {
    const requestId = extractServiceRequestId(log);
    if (requestId != null) return requestId;
  }

  return null;
}

export async function recoverDraftFromReceipt(
  instance: LocalInstance,
  signal: AbortSignal,
): Promise<LocalInstance> {
  if (
    signal.aborted
    || instance.requestId != null
    || !instance.txHash
    || !['creating', 'error'].includes(instance.status)
  ) {
    return instance;
  }

  try {
    const receipt = await publicClient.getTransactionReceipt({
      hash: instance.txHash as `0x${string}`,
    });
    if (signal.aborted) return instance;

    if (receipt.status === 'reverted') {
      return {
        ...instance,
        status: 'error',
        errorMessage: 'Instance service request reverted before activation.',
      };
    }

    const requestId = getRequestIdFromReceiptLogs(
      receipt.logs as Array<{ data: `0x${string}`; topics: readonly `0x${string}`[] }>,
    );
    if (requestId != null) {
      return {
        ...instance,
        status: instance.sandboxId ? instance.status : 'creating',
        requestId,
        errorMessage: undefined,
      };
    }

    return {
      ...instance,
      status: 'error',
      errorMessage: 'Instance transaction confirmed without a ServiceRequested event.',
    };
  } catch {
    if (hasRecentPendingTx(instance)) return instance;
    return {
      ...instance,
      status: 'error',
      errorMessage: 'Instance transaction receipt could not be recovered from the RPC.',
    };
  }
}

async function resolveServiceId(requestId: number, signal: AbortSignal): Promise<string | null> {
  const addrs = getAddresses<SandboxAddresses>();

  try {
    const logs = await publicClient.getLogs({
      address: addrs.services,
      fromBlock: 0n,
      toBlock: 'latest',
    });
    if (signal.aborted) return null;

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
        // Ignore unrelated logs while scanning the chain.
      }
    }
  } catch {
    // Ignore transient RPC failures and keep the local optimistic state.
  }

  return null;
}

export function useInstanceHydration(): InstanceHydrationState {
  const baseUrl = INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL;
  const { getToken, getCachedToken } = useOperatorAuth(baseUrl || undefined);
  const controllerRef = useRef<AbortController | null>(null);
  const [isHydrating, setIsHydrating] = useState(false);
  const [authRequired, setAuthRequired] = useState(false);
  const [lastError, setLastError] = useState<string | null>(null);

  const refresh = useCallback(async ({ interactive = false }: RefreshOpts = {}) => {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    const { signal } = controller;

    setIsHydrating(true);
    setLastError(null);
    setAuthRequired(false);

    try {
      const existing = instanceListStore.get();
      const recoveredExisting = await Promise.all(
        existing.map((instance) => recoverDraftFromReceipt(instance, signal)),
      );
      if (signal.aborted) return false;

      const serviceIdsByRequestId = new Map<number, string>();
      await Promise.all(
        recoveredExisting
          .filter((instance) => instance.requestId != null && !instance.serviceId)
          .map(async (instance) => {
            const requestId = instance.requestId;
            if (requestId == null) return;
            const serviceId = await resolveServiceId(requestId, signal);
            if (serviceId) {
              serviceIdsByRequestId.set(requestId, serviceId);
            }
          }),
      );
      if (signal.aborted) return false;

      const token = interactive ? await getToken() : getCachedToken();
      if (signal.aborted) return false;
      if (!token) {
        setAuthRequired(true);
        return false;
      }

      const apiInstances = await fetchSandboxes(baseUrl, token, '', '', interactive ? getToken : undefined, signal, {
        throwOnError: interactive,
      });
      if (signal.aborted) return false;

      const merged = reconcileInstances(recoveredExisting, apiInstances as ApiSandbox[], serviceIdsByRequestId);
      if (merged.length !== existing.length || merged.some((instance, index) => instance !== existing[index])) {
        instanceListStore.set(merged);
      }

      return true;
    } catch (error) {
      if (signal.aborted) return false;
      const message = error instanceof Error ? error.message : 'Unable to refresh instances';
      setLastError(message);
      if (interactive) {
        toast.error('Unable to refresh instances', {
          description: message,
          duration: 6000,
        });
      }
      return false;
    } finally {
      if (controllerRef.current === controller) {
        controllerRef.current = null;
      }
      if (!signal.aborted) {
        setIsHydrating(false);
      }
    }
  }, [baseUrl, getCachedToken, getToken]);

  useEffect(() => {
    void refresh({ interactive: false });

    return () => {
      controllerRef.current?.abort();
    };
  }, [refresh]);

  return {
    refresh,
    isHydrating,
    authRequired,
    lastError,
  };
}
