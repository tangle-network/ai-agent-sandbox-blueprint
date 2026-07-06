import { type Address } from 'viem';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import { type WorkflowOperatorSummary } from '~/lib/hooks/useWorkflowRuntimeStatus';
import { type PendingWorkflowCreation } from '~/lib/stores/pendingWorkflows';
import { type WorkflowScope } from '~/lib/workflows';
import { type WorkflowRecord } from './types';

export function getWorkflowStatusPresentation(workflow: WorkflowOperatorSummary) {
  if (!workflow.runnable) {
    return {
      label: 'Not Runnable',
      variant: 'stopped' as const,
      detail: workflow.targetStatus === 'missing'
        ? 'Target is no longer available'
        : 'Workflow is currently blocked',
    };
  }

  if (workflow.active) {
    return {
      label: 'Active',
      variant: 'running' as const,
      detail: 'Ready to execute on schedule',
    };
  }

  return {
    label: 'Inactive',
    variant: 'secondary' as const,
    detail: 'Disabled until re-enabled',
  };
}

export function getPendingWorkflowStatusPresentation(pending: PendingWorkflowCreation) {
  switch (pending.status) {
    case 'awaiting-auth':
      return {
        label: 'Submitted',
        variant: 'secondary' as const,
        detail: pending.statusMessage || 'Connect to the operator to verify that the workflow is visible.',
      };
    case 'timed-out':
      return {
        label: 'Still Processing',
        variant: 'secondary' as const,
        detail: pending.statusMessage || 'Creation is taking longer than expected. Check status to look again.',
      };
    case 'processing':
    default:
      return {
        label: 'Processing',
        variant: 'accent' as const,
        detail: pending.statusMessage || 'Transaction confirmed. Waiting for the operator to publish the workflow.',
      };
  }
}

function getWorkflowContractAddress(address: Address): Address | undefined {
  return isContractDeployed(address) ? address : undefined;
}

export function getWorkflowContractAddressForScope(
  addrs: SandboxAddresses,
  scope: WorkflowScope,
): Address | undefined {
  switch (scope) {
    case 'sandbox':
      return getWorkflowContractAddress(addrs.sandboxBlueprint);
    case 'instance':
      return getWorkflowContractAddress(addrs.instanceBlueprint);
    case 'tee':
      return getWorkflowContractAddress(addrs.teeInstanceBlueprint);
  }
}

export function getWorkflowIdentityKey(scope: WorkflowScope, workflowId: bigint | number) {
  return `${scope}:${String(workflowId)}`;
}

export function getWorkflowSortTimestamp(workflow: WorkflowRecord) {
  if (workflow.kind === 'pending') {
    return workflow.pending.createdAt;
  }

  return workflow.data.lastRunAt
    ?? workflow.data.latestExecution?.executedAt
    ?? workflow.data.nextRunAt
    ?? 0;
}

export function getOperatorLabel(scope: WorkflowScope) {
  switch (scope) {
    case 'sandbox':
      return 'Sandbox operator';
    case 'instance':
      return 'Instance operator';
    case 'tee':
      return 'TEE operator';
  }
}

export function workflowStatusTone(variant: 'stopped' | 'running' | 'secondary' | 'accent') {
  if (variant === 'running') return 'ready';
  if (variant === 'accent') return 'brand';
  if (variant === 'stopped') return 'warn';
  return 'muted';
}

export function formatWorkflowDate(value: number | null | undefined, unit: 's' | 'ms') {
  if (value == null || value <= 0) return '--';
  const timestampMs = unit === 's' ? value * 1000 : value;
  return new Date(timestampMs).toLocaleString();
}
