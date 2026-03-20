import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { useStore } from '@nanostores/react';
import {
  getAddresses,
  publicClient,
  selectedChainIdStore,
} from '@tangle-network/blueprint-ui';
import { agentSandboxBlueprintAbi } from '~/lib/contracts/abi';
import type { Address } from 'viem';

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

type WorkflowBatchResult = {
  status: 'success';
  result: WorkflowView;
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
        abi: agentSandboxBlueprintAbi,
        functionName: functionName as any,
        args: args as any,
      }) as Promise<TData>,
    enabled: enabled && !!address,
    refetchInterval,
  });
}

/**
 * Read service-level stats from the blueprint contract.
 */
export function useServiceStats() {
  return useSandboxContractRead<readonly [number, number]>({
    functionName: 'getServiceStats',
    refetchInterval: 15_000,
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
 * Read total active sandboxes count.
 */
export function useTotalActiveSandboxes() {
  return useSandboxContractRead<number>({
    functionName: 'totalActiveSandboxes',
    refetchInterval: 10_000,
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
 * Read an operator's load (active / max capacity).
 */
export function useOperatorLoad(operator: `0x${string}` | undefined) {
  return useSandboxContractRead<readonly [number, number]>({
    functionName: 'getOperatorLoad',
    args: operator ? [operator] : undefined,
    enabled: !!operator,
  });
}

/**
 * Get the default max capacity.
 */
export function useDefaultMaxCapacity() {
  return useSandboxContractRead<number>({
    functionName: 'defaultMaxCapacity',
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

export function useWorkflowIdsForAddress(address: Address | undefined, activeOnly: boolean = false) {
  const chainId = useStore(selectedChainIdStore);

  return useQuery<readonly bigint[], Error>({
    queryKey: ['workflow-contract-read', chainId, address, 'getWorkflowIds', activeOnly],
    queryFn: async () =>
      publicClient.readContract({
        address: address as Address,
        abi: agentSandboxBlueprintAbi,
        functionName: 'getWorkflowIds',
        args: [activeOnly],
      }) as Promise<readonly bigint[]>,
    enabled: !!address,
    refetchInterval: 15_000,
  });
}

/**
 * Get a specific workflow config.
 */
export function useWorkflow(workflowId: bigint | undefined) {
  return useSandboxContractRead<WorkflowConfig>({
    functionName: 'getWorkflow',
    args: workflowId !== undefined ? [workflowId] : undefined,
    enabled: workflowId !== undefined,
  });
}

/**
 * Get pricing info: multiplier for a specific job.
 */
export function useJobPriceMultiplier(jobId: number) {
  return useSandboxContractRead<bigint>({
    functionName: 'getJobPriceMultiplier',
    args: [jobId],
  });
}

/**
 * Get all default job rates for a given base rate.
 */
export function useDefaultJobRates(baseRate: bigint) {
  return useSandboxContractRead<readonly [readonly number[], readonly bigint[]]>({
    functionName: 'getDefaultJobRates',
    args: [baseRate],
  });
}

/**
 * Batch-read multiple workflows by ID.
 */
export function useWorkflowBatch(workflowIds: bigint[]) {
  const { address, chainId } = useSandboxReadDeps();
  return useWorkflowBatchForAddress(address, workflowIds, chainId);
}

export function useWorkflowBatchForAddress(
  address: Address | undefined,
  workflowIds: bigint[],
  chainIdOverride?: number,
) {
  const chainId = useStore(selectedChainIdStore);
  const workflowIdKeys = workflowIds.map((id) => id.toString());
  const effectiveChainId = chainIdOverride ?? chainId;

  return useQuery<WorkflowBatchResult[], Error>({
    queryKey: ['workflow-batch', effectiveChainId, address, workflowIdKeys],
    queryFn: async () =>
      Promise.all(
        workflowIds.map(async (id) => {
          const result = await publicClient.readContract({
            address: address as Address,
            abi: agentSandboxBlueprintAbi,
            functionName: 'getWorkflow' as any,
            args: [id] as any,
          });

          return {
            status: 'success' as const,
            result: normalizeWorkflowConfig(result as unknown as WorkflowConfig),
          };
        }),
      ),
    enabled: workflowIds.length > 0 && !!address,
    refetchInterval: 15_000,
  });
}
