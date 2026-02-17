import { useReadContract, useReadContracts } from 'wagmi';
import { agentSandboxBlueprintAbi } from '~/lib/contracts/abi';
import { getAddresses } from '~/lib/contracts/publicClient';

/**
 * Read service-level stats from the blueprint contract.
 */
export function useServiceStats() {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getServiceStats',
    query: { refetchInterval: 15_000 },
  });
}

/**
 * Read available capacity across all operators.
 */
export function useAvailableCapacity() {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getAvailableCapacity',
    query: { refetchInterval: 15_000 },
  });
}

/**
 * Read total active sandboxes count.
 */
export function useTotalActiveSandboxes() {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'totalActiveSandboxes',
    query: { refetchInterval: 10_000 },
  });
}

/**
 * Check if a specific sandbox is active.
 */
export function useSandboxActive(sandboxId: string | undefined) {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'isSandboxActive',
    args: sandboxId ? [sandboxId] : undefined,
    query: {
      enabled: !!sandboxId,
      refetchInterval: 10_000,
    },
  });
}

/**
 * Get the operator assigned to a sandbox.
 */
export function useSandboxOperator(sandboxId: string | undefined) {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getSandboxOperator',
    args: sandboxId ? [sandboxId] : undefined,
    query: { enabled: !!sandboxId },
  });
}

/**
 * Read an operator's load (active / max capacity).
 */
export function useOperatorLoad(operator: `0x${string}` | undefined) {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getOperatorLoad',
    args: operator ? [operator] : undefined,
    query: { enabled: !!operator },
  });
}

/**
 * Get the default max capacity.
 */
export function useDefaultMaxCapacity() {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'defaultMaxCapacity',
  });
}

/**
 * Get all workflow IDs.
 */
export function useWorkflowIds(activeOnly: boolean = false) {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getWorkflowIds',
    args: [activeOnly],
    query: { refetchInterval: 15_000 },
  });
}

/**
 * Get a specific workflow config.
 */
export function useWorkflow(workflowId: bigint | undefined) {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getWorkflow',
    args: workflowId !== undefined ? [workflowId] : undefined,
    query: { enabled: workflowId !== undefined },
  });
}

/**
 * Get pricing info: multiplier for a specific job.
 */
export function useJobPriceMultiplier(jobId: number) {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getJobPriceMultiplier',
    args: [jobId],
  });
}

/**
 * Get all default job rates for a given base rate.
 */
export function useDefaultJobRates(baseRate: bigint) {
  const addrs = getAddresses();
  return useReadContract({
    address: addrs.sandboxBlueprint,
    abi: agentSandboxBlueprintAbi,
    functionName: 'getDefaultJobRates',
    args: [baseRate],
  });
}

/**
 * Batch-read multiple workflows by ID.
 */
export function useWorkflowBatch(workflowIds: bigint[]) {
  const addrs = getAddresses();
  return useReadContracts({
    contracts: workflowIds.map((id) => ({
      address: addrs.sandboxBlueprint,
      abi: agentSandboxBlueprintAbi,
      functionName: 'getWorkflow' as const,
      args: [id] as const,
    })),
    query: {
      enabled: workflowIds.length > 0,
      refetchInterval: 15_000,
    },
  });
}
