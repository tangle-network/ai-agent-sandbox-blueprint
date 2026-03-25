import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { useStore } from '@nanostores/react';
import {
  getAddresses,
  publicClient,
  selectedChainIdStore,
} from '@tangle-network/blueprint-ui';
import { agentInstanceBlueprintAbi } from '~/lib/contracts/abi';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import type { Address } from 'viem';

type BlueprintType = 'instance' | 'tee-instance';

function useInstanceReadDeps(blueprintType: BlueprintType) {
  const chainId = useStore(selectedChainIdStore);
  const addrs = getAddresses<SandboxAddresses>();
  const address =
    blueprintType === 'tee-instance'
      ? addrs.teeInstanceBlueprint
      : addrs.instanceBlueprint;

  return { address, chainId };
}

function useInstanceContractRead<TData>({
  blueprintType,
  functionName,
  args,
  enabled = true,
  refetchInterval,
}: {
  blueprintType: BlueprintType;
  functionName: string;
  args?: readonly unknown[];
  enabled?: boolean;
  refetchInterval?: number;
}): UseQueryResult<TData, Error> {
  const { address, chainId } = useInstanceReadDeps(blueprintType);

  return useQuery<TData, Error>({
    queryKey: ['instance-contract-read', chainId, address, functionName, args],
    queryFn: async () =>
      publicClient.readContract({
        address,
        abi: agentInstanceBlueprintAbi,
        functionName: functionName as any,
        args: args as any,
      }) as Promise<TData>,
    enabled: enabled && !!address && isContractDeployed(address),
    refetchInterval,
  });
}

export function useIsProvisioned(
  serviceId: bigint,
  blueprintType: BlueprintType,
  enabled = true,
) {
  return useInstanceContractRead<boolean>({
    blueprintType,
    functionName: 'isProvisioned',
    args: [serviceId],
    enabled,
    refetchInterval: 15_000,
  });
}

export function useOperatorCount(
  serviceId: bigint,
  blueprintType: BlueprintType,
  enabled = true,
) {
  return useInstanceContractRead<number>({
    blueprintType,
    functionName: 'getOperatorCount',
    args: [serviceId],
    enabled,
    refetchInterval: 15_000,
  });
}

export function useOperatorEndpoints(
  serviceId: bigint,
  blueprintType: BlueprintType,
  enabled = true,
) {
  return useInstanceContractRead<readonly [readonly Address[], readonly string[]]>({
    blueprintType,
    functionName: 'getOperatorEndpoints',
    args: [serviceId],
    enabled,
    refetchInterval: 15_000,
  });
}

export function useIsOperatorProvisioned(
  serviceId: bigint,
  operator: Address | undefined,
  blueprintType: BlueprintType,
  enabled = true,
) {
  return useInstanceContractRead<boolean>({
    blueprintType,
    functionName: 'isOperatorProvisioned',
    args: operator ? [serviceId, operator] : undefined,
    enabled: enabled && !!operator,
    refetchInterval: 15_000,
  });
}

export function useAttestationHash(
  serviceId: bigint,
  operator: Address | undefined,
  blueprintType: BlueprintType,
  enabled = true,
) {
  return useInstanceContractRead<`0x${string}`>({
    blueprintType,
    functionName: 'getAttestationHash',
    args: operator ? [serviceId, operator] : undefined,
    enabled: enabled && !!operator,
    refetchInterval: 15_000,
  });
}

export function useServiceConfig(
  serviceId: bigint,
  blueprintType: BlueprintType,
  enabled = true,
) {
  return useInstanceContractRead<`0x${string}`>({
    blueprintType,
    functionName: 'getServiceConfig',
    args: [serviceId],
    enabled,
    refetchInterval: 15_000,
  });
}

/**
 * Composite hook for the on-chain verification card.
 */
export function useOnChainVerification({
  serviceId,
  operator,
  blueprintType,
  enabled,
}: {
  serviceId: bigint | null;
  operator: string | undefined;
  blueprintType: BlueprintType;
  enabled: boolean;
}) {
  const isEnabled = enabled && serviceId !== null;
  const sid = serviceId ?? 0n;
  const op = operator as Address | undefined;

  const isProvisioned = useIsProvisioned(sid, blueprintType, isEnabled);
  const operatorCount = useOperatorCount(sid, blueprintType, isEnabled);
  const operatorEndpoints = useOperatorEndpoints(sid, blueprintType, isEnabled);
  const isOperatorProvisioned = useIsOperatorProvisioned(
    sid,
    op,
    blueprintType,
    isEnabled && !!operator,
  );
  const attestationHash = useAttestationHash(
    sid,
    op,
    blueprintType,
    isEnabled && !!operator && blueprintType === 'tee-instance',
  );

  return {
    isProvisioned,
    operatorCount,
    operatorEndpoints,
    isOperatorProvisioned,
    attestationHash,
  };
}
