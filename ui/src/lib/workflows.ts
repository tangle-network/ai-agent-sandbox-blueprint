import type { LocalInstance } from '~/lib/stores/instances';
import type { LocalSandbox } from '~/lib/stores/sandboxes';
import {
  INSTANCE_OPERATOR_API_URL,
  OPERATOR_API_URL,
} from '~/lib/config';

export type WorkflowBlueprintId =
  | 'ai-agent-sandbox-blueprint'
  | 'ai-agent-instance-blueprint'
  | 'ai-agent-tee-instance-blueprint';

export type WorkflowScope = 'sandbox' | 'instance' | 'tee';

export const WORKFLOW_TARGET_SANDBOX = 0;
export const WORKFLOW_TARGET_INSTANCE = 1;

export function getWorkflowScopeFromBlueprintId(
  blueprintId: WorkflowBlueprintId,
): WorkflowScope {
  switch (blueprintId) {
    case 'ai-agent-sandbox-blueprint':
      return 'sandbox';
    case 'ai-agent-instance-blueprint':
      return 'instance';
    case 'ai-agent-tee-instance-blueprint':
      return 'tee';
  }
}

export function getWorkflowBlueprintIdForScope(
  scope: WorkflowScope,
): WorkflowBlueprintId {
  switch (scope) {
    case 'sandbox':
      return 'ai-agent-sandbox-blueprint';
    case 'instance':
      return 'ai-agent-instance-blueprint';
    case 'tee':
      return 'ai-agent-tee-instance-blueprint';
  }
}

export function getWorkflowOperatorUrl(scope: WorkflowScope): string {
  return scope === 'sandbox'
    ? OPERATOR_API_URL
    : (INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL);
}

export function buildWorkflowDetailPath(
  scope: WorkflowScope,
  workflowId: bigint | number | string,
): string {
  return `/workflows/${scope}/${String(workflowId)}`;
}

export function resolveWorkflowTargetLabelFromValues(
  targetKind: number | null | undefined,
  targetSandboxId: string | null | undefined,
  targetServiceId: string | number | null | undefined,
  blueprintId: WorkflowBlueprintId,
  sandboxes: LocalSandbox[],
  instances: LocalInstance[],
) {
  if (targetKind == null) {
    return { label: 'Resolving target...', kindLabel: 'Workflow' };
  }

  if (targetKind === WORKFLOW_TARGET_SANDBOX) {
    const sandbox = sandboxes.find((record) => record.sandboxId === targetSandboxId);
    return {
      label: sandbox?.name ?? targetSandboxId ?? 'Unknown sandbox',
      kindLabel: 'Sandbox',
    };
  }

  const instance = instances.find((record) => {
    if (record.serviceId !== String(targetServiceId ?? '')) return false;
    if (blueprintId === 'ai-agent-tee-instance-blueprint') return !!record.teeEnabled;
    if (blueprintId === 'ai-agent-instance-blueprint') return !record.teeEnabled;
    return true;
  });

  return {
    label: instance?.name ?? `Service #${String(targetServiceId ?? '0')}`,
    kindLabel: blueprintId === 'ai-agent-tee-instance-blueprint' ? 'TEE Instance' : 'Instance',
  };
}
