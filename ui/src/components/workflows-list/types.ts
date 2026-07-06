import { type WorkflowOperatorSummary } from '~/lib/hooks/useWorkflowRuntimeStatus';
import { type PendingWorkflowCreation } from '~/lib/stores/pendingWorkflows';
import { type WorkflowBlueprintId, type WorkflowScope } from '~/lib/workflows';

export type RemoteWorkflowRecord = {
  kind: 'remote';
  id: bigint;
  scope: WorkflowScope;
  blueprintId: WorkflowBlueprintId;
  data: WorkflowOperatorSummary;
  targetLabel: string;
  kindLabel: string;
};

export type PendingWorkflowRecord = {
  kind: 'pending';
  id: bigint;
  scope: WorkflowScope;
  blueprintId: WorkflowBlueprintId;
  pending: PendingWorkflowCreation;
  targetLabel: string;
  kindLabel: string;
};

export type WorkflowRecord = RemoteWorkflowRecord | PendingWorkflowRecord;
