import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { useStore } from '@nanostores/react';
import {
  getAddresses,
  publicClient,
  selectedChainIdStore,
} from '@tangle-network/blueprint-ui';
import { agentSandboxBlueprintAbi } from '~/lib/contracts/abi';
import type { Abi, Address } from 'viem';

function useSandboxReadDeps() {
  const chainId = useStore(selectedChainIdStore);
  const addrs = getAddresses();

  return {
    address: addrs.sandboxBlueprint,
    chainId,
  };
}

interface WorkflowConfig {
  name: string;
  workflow_json: string;
  trigger_type: string;
  trigger_config: string;
  sandbox_config_json: string;
  target_kind: number;
  target_sandbox_id: string;
  target_service_id: bigint;
  active: boolean;
  created_at: bigint;
  updated_at: bigint;
  last_triggered_at: bigint;
}

export interface WorkflowView {
  name: string;
  workflow_json: string;
  trigger_type: string;
  trigger_config: string;
  sandbox_config_json: string;
  target_kind: number;
  target_sandbox_id: string;
  target_service_id: string;
  active: boolean;
  created_at: number;
  updated_at: number;
  last_triggered_at: number;
}

type WorkflowOwnershipEvent = {
  callId: bigint;
  caller: Address;
};

export function normalizeWorkflowConfig(workflow: WorkflowConfig): WorkflowView {
  return {
    ...workflow,
    target_service_id: workflow.target_service_id.toString(),
    created_at: Number(workflow.created_at),
    updated_at: Number(workflow.updated_at),
    last_triggered_at: Number(workflow.last_triggered_at),
  };
}

function useSandboxContractRead<TData>({
  functionName,
  args,
  enabled = true,
  refetchInterval,
}: {
  functionName: string;
  args?: readonly unknown[];
  enabled?: boolean;
  refetchInterval?: number;
}): UseQueryResult<TData, Error> {
  const { address, chainId } = useSandboxReadDeps();

  return useQuery<TData, Error>({
    queryKey: ['sandbox-contract-read', chainId, address, functionName, args],
    queryFn: async () =>
      publicClient.readContract({
        address,
        // This generic helper takes a runtime `functionName`/`args`, so viem
        // cannot infer the return type. Widening the ABI to `Abi` here avoids
        // viem's per-function overload inference (which hits TS2589 on the
        // full ~100-entry contract ABI) while the caller-facing `TData`
        // generic carries the actual return shape.
        abi: agentSandboxBlueprintAbi as Abi,
        functionName,
        args,
      }) as Promise<TData>,
    enabled: enabled && !!address,
    refetchInterval,
  });
}

/**
 * Read available capacity across all operators.
 */
export function useAvailableCapacity() {
  return useSandboxContractRead<number>({
    functionName: 'getAvailableCapacity',
    refetchInterval: 15_000,
  });
}

/**
 * Check if a specific sandbox is active.
 */
export function useSandboxActive(sandboxId: string | undefined) {
  return useSandboxContractRead<boolean>({
    functionName: 'isSandboxActive',
    args: sandboxId ? [sandboxId] : undefined,
    enabled: !!sandboxId,
    refetchInterval: 10_000,
  });
}

/**
 * Get the operator assigned to a sandbox.
 */
export function useSandboxOperator(sandboxId: string | undefined) {
  return useSandboxContractRead<`0x${string}`>({
    functionName: 'getSandboxOperator',
    args: sandboxId ? [sandboxId] : undefined,
    enabled: !!sandboxId,
  });
}

/**
 * Get all workflow IDs.
 */
export function useWorkflowIds(activeOnly: boolean = false) {
  return useSandboxContractRead<readonly bigint[]>({
    functionName: 'getWorkflowIds',
    args: [activeOnly],
    refetchInterval: 15_000,
  });
}

export function filterOwnedWorkflowIds(
  workflowIds: readonly bigint[],
  ownershipEvents: readonly WorkflowOwnershipEvent[],
  owner: Address,
): readonly bigint[] {
  const normalizedOwner = owner.toLowerCase();
  const ownedWorkflowIds = new Set(
    ownershipEvents
      .filter((event) => event.caller.toLowerCase() === normalizedOwner)
      .map((event) => event.callId.toString()),
  );

  return workflowIds.filter((workflowId) => ownedWorkflowIds.has(workflowId.toString()));
}

export function useWorkflowForAddress(
  address: Address | undefined,
  workflowId: bigint | undefined,
  chainIdOverride?: number,
) {
  const chainId = useStore(selectedChainIdStore);
  const effectiveChainId = chainIdOverride ?? chainId;

  return useQuery<WorkflowView, Error>({
    queryKey: [
      'workflow-contract-read',
      effectiveChainId,
      address,
      'getWorkflow',
      workflowId?.toString() ?? null,
    ],
    queryFn: async () => {
      const result = await publicClient.readContract({
        address: address as Address,
        abi: agentSandboxBlueprintAbi,
        functionName: 'getWorkflow',
        args: [workflowId as bigint],
      });

      return normalizeWorkflowConfig(result as unknown as WorkflowConfig);
    },
    enabled: !!address && workflowId !== undefined,
    refetchInterval: 15_000,
  });
}

