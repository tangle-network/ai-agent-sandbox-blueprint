import { Link } from 'react-router';
import { type PendingWorkflowCreation } from '~/lib/stores/pendingWorkflows';
import { buildWorkflowDetailPath } from '~/lib/workflows';
import { ConsoleChip } from '~/components/console/ConsolePrimitives';
import {
  IdentityMark,
  getBlueprintIdentity,
} from '~/components/shared/VisualIdentity';
import { WorkflowActionButton } from './WorkflowActionButton';
import {
  formatWorkflowDate,
  getPendingWorkflowStatusPresentation,
  getWorkflowStatusPresentation,
  workflowStatusTone,
} from './helpers';
import { type RemoteWorkflowRecord, type WorkflowRecord } from './types';

export function WorkflowTable({
  workflows,
  onTrigger,
  onCancel,
  onResolvePending,
  resolvingPendingKeys,
  txPending,
}: {
  workflows: WorkflowRecord[];
  onTrigger: (workflow: RemoteWorkflowRecord) => void;
  onCancel: (workflow: RemoteWorkflowRecord) => void;
  onResolvePending: (pending: PendingWorkflowCreation) => void;
  resolvingPendingKeys: Record<string, boolean>;
  txPending: boolean;
}) {
  return (
    <div className="overflow-auto">
      <table className="min-w-[980px] w-full table-fixed border-collapse">
        <colgroup>
          <col className="w-[25%]" />
          <col className="w-[12%]" />
          <col className="w-[12%]" />
          <col className="w-[18%]" />
          <col className="w-[13%]" />
          <col className="w-[10%]" />
          <col className="w-[10%]" />
        </colgroup>
        <thead>
          <tr className="border-b border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-surface)]">
            {['Workflow', 'Status', 'Trigger', 'Target', 'Last run', 'Next run', 'Actions'].map((label) => (
              <th
                key={label}
                className="px-3 py-2 text-left font-data text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--sandbox-console-muted)]"
              >
                {label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {workflows.map((workflow) => (
            <WorkflowTableRow
              key={`${workflow.kind}:${workflow.blueprintId}:${String(workflow.id)}`}
              workflow={workflow}
              onTrigger={onTrigger}
              onCancel={onCancel}
              onResolvePending={onResolvePending}
              pendingActionLoading={workflow.kind === 'pending' && !!resolvingPendingKeys[workflow.pending.key]}
              txPending={txPending}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function WorkflowTableRow({
  workflow,
  onTrigger,
  onCancel,
  onResolvePending,
  pendingActionLoading,
  txPending,
}: {
  workflow: WorkflowRecord;
  onTrigger: (workflow: RemoteWorkflowRecord) => void;
  onCancel: (workflow: RemoteWorkflowRecord) => void;
  onResolvePending: (pending: PendingWorkflowCreation) => void;
  pendingActionLoading: boolean;
  txPending: boolean;
}) {
  const triggerLabel: Record<string, string> = {
    cron: 'Cron',
    manual: 'Manual',
  };

  const isPending = workflow.kind === 'pending';
  const name = isPending ? workflow.pending.name : workflow.data.name;
  const triggerType = isPending ? workflow.pending.triggerType : workflow.data.triggerType;
  const triggerConfig = isPending ? workflow.pending.triggerConfig : workflow.data.triggerConfig;
  const status = isPending
    ? getPendingWorkflowStatusPresentation(workflow.pending)
    : getWorkflowStatusPresentation(workflow.data);
  const detailPath = isPending ? null : buildWorkflowDetailPath(workflow.scope, workflow.id);
  const canTrigger = !isPending && workflow.data.runnable && workflow.data.targetServiceId !== 0;
  const canCancel = !isPending && workflow.data.active && workflow.data.targetServiceId !== 0;

  return (
    <tr className="group border-b border-[var(--sandbox-console-border)] transition-colors hover:bg-[var(--sandbox-console-surface)]">
      <td className="px-3 py-3">
        <div className="flex min-w-0 items-center gap-3">
          <IdentityMark identity={getBlueprintIdentity(workflow.blueprintId)} size="md" />
          <span className="min-w-0">
            {detailPath ? (
              <Link
                to={detailPath}
                className="block truncate font-display text-sm font-bold text-[var(--sandbox-console-text)] transition-colors hover:text-[var(--sandbox-console-brand)]"
              >
                {name || `Workflow #${String(workflow.id)}`}
              </Link>
            ) : (
              <span className="block truncate font-display text-sm font-bold text-[var(--sandbox-console-text)]">
                {name || `Workflow #${String(workflow.id)}`}
              </span>
            )}
            <span className="block truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
              {workflow.kindLabel} · #{String(workflow.id)}
            </span>
          </span>
        </div>
      </td>
      <td className="px-3 py-3">
        <ConsoleChip tone={workflowStatusTone(status.variant)}>{status.label}</ConsoleChip>
      </td>
      <td className="px-3 py-3">
        <span className="block truncate font-display text-sm font-bold text-[var(--sandbox-console-text)]">
          {triggerLabel[triggerType] ?? triggerType}
        </span>
        <span className="block truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
          {triggerConfig || 'manual'}
        </span>
      </td>
      <td className="px-3 py-3">
        <span className="block truncate font-data text-xs font-bold text-[var(--sandbox-console-text)]">
          {isPending
            ? workflow.targetLabel
            : workflow.data.targetStatus === 'missing'
              ? `Target missing: ${workflow.targetLabel}`
              : workflow.targetLabel}
        </span>
        <span className="block truncate font-data text-[11px] text-[var(--sandbox-console-subtle)]">
          {status.detail}
        </span>
      </td>
      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">
        {isPending ? formatWorkflowDate(workflow.pending.submittedAt, 'ms') : formatWorkflowDate(workflow.data.lastRunAt, 's')}
      </td>
      <td className="px-3 py-3 font-data text-xs text-[var(--sandbox-console-muted)]">
        {isPending ? 'pending' : formatWorkflowDate(workflow.data.nextRunAt, 's')}
      </td>
      <td className="px-3 py-3">
        <div className="flex flex-wrap items-center gap-2">
          {isPending ? (
            <WorkflowActionButton
              tone="secondary"
              onClick={() => onResolvePending(workflow.pending)}
              disabled={pendingActionLoading}
            >
              {pendingActionLoading
                ? 'Checking...'
                : workflow.pending.status === 'awaiting-auth'
                  ? 'Connect Operator'
                  : 'Check Status'}
            </WorkflowActionButton>
          ) : null}
          {!isPending && workflow.data.active && workflow.data.targetServiceId !== 0 ? (
            <WorkflowActionButton
              tone="success"
              onClick={() => onTrigger(workflow)}
              disabled={txPending || !canTrigger}
              icon="i-ph:play-bold"
            >
              Trigger
            </WorkflowActionButton>
          ) : null}
          {!isPending && canCancel ? (
            <WorkflowActionButton
              tone="secondary"
              onClick={() => onCancel(workflow)}
              disabled={txPending}
              icon="i-ph:stop-bold"
            >
              Cancel
            </WorkflowActionButton>
          ) : null}
        </div>
      </td>
    </tr>
  );
}
