import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useStore } from '@nanostores/react';
import type { Address } from 'viem';
import {
  getAddresses,
  publicClient,
  selectedChainIdStore,
  tangleOperatorsAbi,
  tangleServicesAbi,
  useOperators,
  type DiscoveredOperator,
} from '@tangle-network/blueprint-ui';
import {
  INSTANCE_ONCHAIN_BLUEPRINT_ID,
  INSTANCE_ONCHAIN_SERVICE_ID,
  INSTANCE_OPERATOR_API_URL,
  OPERATOR_API_URL,
  SANDBOX_ONCHAIN_BLUEPRINT_ID,
  SANDBOX_ONCHAIN_SERVICE_ID,
  TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
  TEE_INSTANCE_ONCHAIN_SERVICE_ID,
} from '~/lib/config';
import type { SandboxAddresses } from '~/lib/contracts/chains';

type ReliableOperatorSource = 'event-index' | 'service-membership' | 'count-only' | 'none';

export interface ReliableOperatorsResult {
  operators: DiscoveredOperator[];
  isLoading: boolean;
  error: Error | null;
  operatorCount: bigint;
  countError: Error | null;
  listError: Error | null;
  source: ReliableOperatorSource;
  listIncomplete: boolean;
}

function parseBlueprintId(value: string | bigint | number | undefined): bigint {
  try {
    return BigInt(value || 0);
  } catch {
    return 0n;
  }
}

function configuredServiceIds(): bigint[] {
  const ids = [
    SANDBOX_ONCHAIN_SERVICE_ID,
    INSTANCE_ONCHAIN_SERVICE_ID,
    TEE_INSTANCE_ONCHAIN_SERVICE_ID,
  ]
    .map(parseBlueprintId)
    .filter((id) => id > 0n);

  return Array.from(new Set(ids.map((id) => id.toString()))).map((id) => BigInt(id));
}

function fallbackRpcAddress(blueprintId: bigint): string {
  if (
    blueprintId.toString() === INSTANCE_ONCHAIN_BLUEPRINT_ID
    || blueprintId.toString() === TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID
  ) {
    return INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL;
  }

  if (blueprintId.toString() === SANDBOX_ONCHAIN_BLUEPRINT_ID) {
    return OPERATOR_API_URL;
  }

  return OPERATOR_API_URL;
}

function readPreferenceValue(
  value: unknown,
  field: 'ecdsaPublicKey' | 'rpcAddress',
): string {
  if (Array.isArray(value)) {
    return String(value[field === 'ecdsaPublicKey' ? 0 : 1] ?? '');
  }

  if (value && typeof value === 'object') {
    return String((value as Record<string, unknown>)[field] ?? '');
  }

  return '';
}

function normalizeOperators(operators: readonly Address[]): Address[] {
  const byAddress = new Map<string, Address>();
  for (const operator of operators) {
    byAddress.set(operator.toLowerCase(), operator);
  }
  return Array.from(byAddress.values());
}

export function useReliableOperators(blueprintIdValue: string | bigint | number | undefined): ReliableOperatorsResult {
  const chainId = useStore(selectedChainIdStore);
  const blueprintId = parseBlueprintId(blueprintIdValue);
  const enabled = blueprintId > 0n;
  const addrs = getAddresses<SandboxAddresses>();
  const indexed = useOperators(enabled ? blueprintId : 0n);

  const countQuery = useQuery<bigint, Error>({
    queryKey: ['reliable-operator-count', chainId, addrs.services, blueprintId.toString()],
    enabled,
    staleTime: 15_000,
    refetchInterval: 15_000,
    queryFn: async () => publicClient.readContract({
      address: addrs.services,
      abi: tangleOperatorsAbi,
      functionName: 'blueprintOperatorCount',
      args: [blueprintId],
    }) as Promise<bigint>,
  });

  const serviceCandidatesQuery = useQuery<Address[], Error>({
    queryKey: ['reliable-operator-service-candidates', chainId, addrs.services, configuredServiceIds().map(String)],
    enabled,
    staleTime: 15_000,
    refetchInterval: 15_000,
    queryFn: async () => {
      const results = await Promise.allSettled(
        configuredServiceIds().map((serviceId) =>
          publicClient.readContract({
            address: addrs.services,
            abi: tangleServicesAbi,
            functionName: 'getServiceOperators',
            args: [serviceId],
          }) as Promise<readonly Address[]>,
        ),
      );

      return normalizeOperators(results.flatMap((result) => result.status === 'fulfilled' ? result.value : []));
    },
  });

  const operatorCount = countQuery.data ?? indexed.operatorCount;
  const needsMembershipFallback = enabled && indexed.operators.length === 0 && operatorCount > 0n;
  const candidateOperators = serviceCandidatesQuery.data ?? [];

  const membershipQuery = useQuery<DiscoveredOperator[], Error>({
    queryKey: [
      'reliable-operator-membership',
      chainId,
      addrs.services,
      blueprintId.toString(),
      candidateOperators.map((operator) => operator.toLowerCase()).sort(),
    ],
    enabled: needsMembershipFallback && candidateOperators.length > 0,
    staleTime: 15_000,
    refetchInterval: 15_000,
    queryFn: async () => {
      const results = await Promise.allSettled(
        candidateOperators.map(async (operator): Promise<DiscoveredOperator | null> => {
          const registered = await publicClient.readContract({
            address: addrs.services,
            abi: tangleOperatorsAbi,
            functionName: 'isOperatorRegistered',
            args: [blueprintId, operator],
          });

          if (registered !== true) return null;

          try {
            const preferences = await publicClient.readContract({
              address: addrs.services,
              abi: tangleOperatorsAbi,
              functionName: 'getOperatorPreferences',
              args: [blueprintId, operator],
            });

            return {
              address: operator,
              ecdsaPublicKey: readPreferenceValue(preferences, 'ecdsaPublicKey') || '0x',
              rpcAddress: readPreferenceValue(preferences, 'rpcAddress') || fallbackRpcAddress(blueprintId),
            };
          } catch {
            return {
              address: operator,
              ecdsaPublicKey: '0x',
              rpcAddress: fallbackRpcAddress(blueprintId),
            };
          }
        }),
      );

      return results
        .filter((result): result is PromiseFulfilledResult<DiscoveredOperator | null> => result.status === 'fulfilled')
        .map((result) => result.value)
        .filter((operator): operator is DiscoveredOperator => operator != null);
    },
  });

  const operators = indexed.operators.length > 0
    ? indexed.operators
    : (membershipQuery.data ?? []);

  const listError = indexed.error ?? membershipQuery.error ?? null;
  const source = useMemo<ReliableOperatorSource>(() => {
    if (indexed.operators.length > 0) return 'event-index';
    if ((membershipQuery.data?.length ?? 0) > 0) return 'service-membership';
    if (operatorCount > 0n) return 'count-only';
    return 'none';
  }, [indexed.operators.length, membershipQuery.data?.length, operatorCount]);

  return {
    operators: enabled ? operators : [],
    isLoading: enabled && operators.length === 0 && (
      indexed.isLoading
      || countQuery.isLoading
      || serviceCandidatesQuery.isLoading
      || membershipQuery.isLoading
    ),
    error: enabled && operators.length === 0 ? (listError ?? countQuery.error ?? null) : null,
    operatorCount: enabled ? operatorCount : 0n,
    countError: enabled ? countQuery.error ?? null : null,
    listError: enabled ? listError : null,
    source: enabled ? source : 'none',
    listIncomplete: enabled ? operatorCount > BigInt(operators.length) : false,
  };
}
