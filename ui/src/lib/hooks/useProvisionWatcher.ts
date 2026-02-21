import { useState, useCallback, useRef, useEffect } from 'react';
import { useWatchContractEvent } from 'wagmi';
import { agentInstanceBlueprintAbi } from '~/lib/contracts/abi';
import { getAddresses } from '~/lib/contracts/publicClient';
import type { SandboxAddresses } from '~/lib/contracts/chains';

interface ProvisionResult {
  sandboxId: string;
  sidecarUrl: string;
}

/**
 * Watch for OperatorProvisioned events on the Instance BSM contract.
 * Used for instance and TEE-instance blueprints where the sidecar URL
 * is emitted as an event (no operator API provision polling).
 */
export function useInstanceProvisionWatcher(
  serviceId: bigint | null,
  blueprintType: 'instance' | 'tee-instance',
  enabled: boolean,
): ProvisionResult | null {
  const [result, setResult] = useState<ProvisionResult | null>(null);
  const resultRef = useRef<ProvisionResult | null>(null);

  const addrs = getAddresses<SandboxAddresses>();
  const address = blueprintType === 'tee-instance'
    ? addrs.teeInstanceBlueprint
    : addrs.instanceBlueprint;

  const onLogs = useCallback((logs: any[]) => {
    if (resultRef.current) return; // Already got a result
    for (const log of logs) {
      const args = log.args as {
        serviceId: bigint;
        operator: `0x${string}`;
        sandboxId: string;
        sidecarUrl: string;
      };
      if (serviceId != null && args.serviceId === serviceId && args.sidecarUrl) {
        const r = { sandboxId: args.sandboxId, sidecarUrl: args.sidecarUrl };
        resultRef.current = r;
        setResult(r);
        return;
      }
    }
  }, [serviceId]);

  useWatchContractEvent({
    address,
    abi: agentInstanceBlueprintAbi,
    eventName: 'OperatorProvisioned',
    enabled: enabled && serviceId != null && !resultRef.current,
    onLogs,
  });

  return result;
}
