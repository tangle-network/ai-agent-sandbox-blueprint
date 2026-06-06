import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Link } from 'react-router';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { useQueryClient } from '@tanstack/react-query';
import { useAccount } from 'wagmi';
import {
  useWorkflowOperatorAccess,
  useWorkflowSummaries,
  type WorkflowOperatorSummary,
} from '~/lib/hooks/useWorkflowRuntimeStatus';
import { getAddresses, useSubmitJob } from '@tangle-network/blueprint-ui';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { getJobById } from '@tangle-network/blueprint-ui';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { cn } from '@tangle-network/blueprint-ui';
import { type Address } from 'viem';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { sandboxListStore } from '~/lib/stores/sandboxes';
import { instanceListStore } from '~/lib/stores/instances';
import {
  normalizeWorkflowOwnerAddress,
  pendingWorkflowStore,
  removePendingWorkflow,
  updatePendingWorkflow,
  type PendingWorkflowCreation,
} from '~/lib/stores/pendingWorkflows';
import {
  buildWorkflowDetailPath,
  getWorkflowBlueprintIdForScope,
  resolveWorkflowTargetLabelFromValues,
  type WorkflowBlueprintId,
  type WorkflowScope,
} from '~/lib/workflows';
import {
  ConsoleChip,
  ConsoleMetricStrip,
  ConsolePage,
  ConsoleSection,
  EmptyConsoleState,
  type ConsoleMetric,
} from '~/components/console/ConsolePrimitives';
import {
  IdentityMark,
  getBlueprintIdentity,
} from '~/components/shared/VisualIdentity';

const WORKFLOW_VISIBILITY_POLL_INTERVAL_MS = 3_000;
const WORKFLOW_VISIBILITY_TIMEOUT_MS = 120_000;

type RemoteWorkflowRecord = {
  kind: 'remote';
  id: bigint;
  scope: WorkflowScope;
  blueprintId: WorkflowBlueprintId;
  data: WorkflowOperatorSummary;
  targetLabel: string;
  kindLabel: string;
};

type PendingWorkflowRecord = {
  kind: 'pending';
  id: bigint;
  scope: WorkflowScope;
  blueprintId: WorkflowBlueprintId;
  pending: PendingWorkflowCreation;
  targetLabel: string;
  kindLabel: string;
};

type WorkflowRecord = RemoteWorkflowRecord | PendingWorkflowRecord;

function getWorkflowStatusPresentation(workflow: WorkflowOperatorSummary) {
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

function getPendingWorkflowStatusPresentation(pending: PendingWorkflowCreation) {
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

function getWorkflowContractAddressForScope(
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

function getWorkflowIdentityKey(scope: WorkflowScope, workflowId: bigint | number) {
  return `${scope}:${String(workflowId)}`;
}

function getWorkflowSortTimestamp(workflow: WorkflowRecord) {
  if (workflow.kind === 'pending') {
    return workflow.pending.createdAt;
  }

  return workflow.data.lastRunAt
    ?? workflow.data.latestExecution?.executedAt
    ?? workflow.data.nextRunAt
    ?? 0;
}

function getOperatorLabel(scope: WorkflowScope) {
  switch (scope) {
    case 'sandbox':
      return 'Sandbox operator';
    case 'instance':
      return 'Instance operator';
    case 'tee':
      return 'TEE operator';
  }
}

export default function Workflows() {
  const queryClient = useQueryClient();
  const { address } = useAccount();
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
  const pendingWorkflowEntries = useStore(pendingWorkflowStore);
  const { submitJob, status: txStatus } = useSubmitJob();
  const addrs = getAddresses<SandboxAddresses>();
  const sandboxOperatorUrl = OPERATOR_API_URL;
  const instanceOperatorUrl = INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL;
  const hasDedicatedInstanceOperator = instanceOperatorUrl !== sandboxOperatorUrl;
  const sandboxWorkflowSummaries = useWorkflowSummaries(sandboxOperatorUrl);
  const instanceWorkflowSummaries = useWorkflowSummaries(
    instanceOperatorUrl,
    hasDedicatedInstanceOperator,
  );
  const sandboxWorkflowAccess = useWorkflowOperatorAccess(sandboxOperatorUrl);
  const instanceWorkflowAccess = useWorkflowOperatorAccess(instanceOperatorUrl);

  const [resolvingPendingKeys, setResolvingPendingKeys] = useState<Record<string, boolean>>({});
  const resolvingPendingKeysRef = useRef(new Set<string>());
  const workflowOperatorAccessRef = useRef({
    sandbox: sandboxWorkflowAccess,
    instance: instanceWorkflowAccess,
  });

  useEffect(() => {
    workflowOperatorAccessRef.current = {
      sandbox: sandboxWorkflowAccess,
      instance: instanceWorkflowAccess,
    };
  }, [instanceWorkflowAccess, sandboxWorkflowAccess]);

  const normalizedOwnerAddress = useMemo(
    () => normalizeWorkflowOwnerAddress(address),
    [address],
  );

  const remoteWorkflows = useMemo<RemoteWorkflowRecord[]>(() => {
    const summaryEntries = [
      ...(sandboxWorkflowSummaries.data ?? []).map((workflow) => ({
        workflow,
        operatorUrl: sandboxOperatorUrl,
      })),
      ...(hasDedicatedInstanceOperator
        ? (instanceWorkflowSummaries.data ?? []).map((workflow) => ({
          workflow,
          operatorUrl: instanceOperatorUrl,
        }))
        : []),
    ];

    const deduped = new Map<string, { workflow: WorkflowOperatorSummary; operatorUrl: string }>();
    for (const entry of summaryEntries) {
      deduped.set(getWorkflowIdentityKey(entry.workflow.scope, entry.workflow.workflowId), entry);
    }

    return Array.from(deduped.values())
      .map(({ workflow }) => {
        const blueprintId = getWorkflowBlueprintIdForScope(workflow.scope);
        const contractAddress = getWorkflowContractAddressForScope(addrs, workflow.scope);
        if (!contractAddress) return null;

        const resolvedTarget = resolveWorkflowTargetLabelFromValues(
          workflow.targetKind,
          workflow.targetSandboxId,
          workflow.targetServiceId,
          blueprintId,
          sandboxes,
          instances,
        );

        return {
          kind: 'remote' as const,
          id: BigInt(workflow.workflowId),
          scope: workflow.scope,
          blueprintId,
          data: workflow,
          targetLabel: resolvedTarget.label,
          kindLabel: resolvedTarget.kindLabel,
        };
      })
      .filter((workflow): workflow is RemoteWorkflowRecord => workflow !== null);
  }, [
    addrs,
    hasDedicatedInstanceOperator,
    instances,
    sandboxes,
    sandboxOperatorUrl,
    sandboxWorkflowSummaries.data,
    instanceOperatorUrl,
    instanceWorkflowSummaries.data,
  ]);

  const remoteWorkflowKeys = useMemo(
    () => new Set(remoteWorkflows.map((workflow) => getWorkflowIdentityKey(workflow.scope, workflow.id))),
    [remoteWorkflows],
  );

  const ownedPendingWorkflows = useMemo(
    () => pendingWorkflowEntries.filter((entry) => entry.ownerAddress === normalizedOwnerAddress),
    [normalizedOwnerAddress, pendingWorkflowEntries],
  );

  useEffect(() => {
    for (const pending of ownedPendingWorkflows) {
      if (remoteWorkflowKeys.has(getWorkflowIdentityKey(pending.scope, pending.workflowId))) {
        removePendingWorkflow(pending.key);
      }
    }
  }, [ownedPendingWorkflows, remoteWorkflowKeys]);

  const workflows = useMemo<WorkflowRecord[]>(() => {
    const merged: WorkflowRecord[] = [...remoteWorkflows];

    for (const pending of ownedPendingWorkflows) {
      if (remoteWorkflowKeys.has(getWorkflowIdentityKey(pending.scope, pending.workflowId))) {
        continue;
      }

      merged.push({
        kind: 'pending',
        id: BigInt(pending.workflowId),
        scope: pending.scope,
        blueprintId: pending.blueprintId,
        pending,
        targetLabel: pending.targetLabel,
        kindLabel: pending.kindLabel,
      });
    }

    return merged.sort((left, right) => {
      const timestampDifference = getWorkflowSortTimestamp(right) - getWorkflowSortTimestamp(left);
      if (timestampDifference !== 0) return timestampDifference;
      return Number(right.id - left.id);
    });
  }, [ownedPendingWorkflows, remoteWorkflowKeys, remoteWorkflows]);

  const isLoading = !!address && (
    sandboxWorkflowSummaries.isLoading
    || (hasDedicatedInstanceOperator && instanceWorkflowSummaries.isLoading)
  );

  const operatorAuthPrompts = useMemo(() => {
    const prompts = [{
      key: sandboxOperatorUrl,
      label: 'Sandbox operator',
      query: sandboxWorkflowSummaries,
    }];

    if (hasDedicatedInstanceOperator) {
      prompts.push({
        key: instanceOperatorUrl,
        label: 'Instance operator',
        query: instanceWorkflowSummaries,
      });
    }

    return prompts.filter((entry) => entry.query.authRequired || entry.query.authError);
  }, [
    hasDedicatedInstanceOperator,
    sandboxOperatorUrl,
    sandboxWorkflowSummaries,
    instanceOperatorUrl,
    instanceWorkflowSummaries,
  ]);

  const operatorErrors = useMemo(
    () => [
      sandboxWorkflowSummaries.error
        ? { key: `error:${sandboxOperatorUrl}`, label: 'Sandbox operator', message: sandboxWorkflowSummaries.error.message }
        : null,
      hasDedicatedInstanceOperator && instanceWorkflowSummaries.error
        ? { key: `error:${instanceOperatorUrl}`, label: 'Instance operator', message: instanceWorkflowSummaries.error.message }
        : null,
    ].filter((entry): entry is { key: string; label: string; message: string } => entry !== null),
    [
      hasDedicatedInstanceOperator,
      sandboxOperatorUrl,
      sandboxWorkflowSummaries.error,
      instanceOperatorUrl,
      instanceWorkflowSummaries.error,
    ],
  );
  const workflowMetrics: ConsoleMetric[] = [
    {
      label: 'Workflows',
      value: String(workflows.length),
      detail: address ? 'owner scoped' : 'wallet gated',
      tone: workflows.length > 0 ? 'brand' : 'muted',
    },
    {
      label: 'Runnable',
      value: String(workflows.filter((workflow) => workflow.kind === 'remote' && workflow.data.runnable).length),
      detail: 'operator ready',
      tone: 'ready',
    },
    {
      label: 'Pending visibility',
      value: String(workflows.filter((workflow) => workflow.kind === 'pending').length),
      detail: 'local receipts',
      tone: 'warn',
    },
    {
      label: 'Operator errors',
      value: String(operatorErrors.length),
      detail: operatorAuthPrompts.length > 0 ? `${operatorAuthPrompts.length} auth` : 'connected',
      tone: operatorErrors.length > 0 ? 'danger' : operatorAuthPrompts.length > 0 ? 'warn' : 'ready',
    },
  ];

  const jobValue = (jobId: number): bigint =>
    BigInt(PRICING_TIERS[jobId]?.multiplier ?? 1) * 1_000_000_000_000_000n;

  const invalidateWorkflowQueries = useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ['workflow-summaries'] }),
      queryClient.invalidateQueries({ queryKey: ['workflow-operator-detail'] }),
      queryClient.invalidateQueries({ queryKey: ['workflow-runtime-status'] }),
      queryClient.invalidateQueries({ queryKey: ['workflow-contract-read'] }),
      queryClient.invalidateQueries({ queryKey: ['workflow-batch'] }),
      queryClient.invalidateQueries({ queryKey: ['sandbox-contract-read'] }),
      queryClient.invalidateQueries({ queryKey: ['sandbox-workflow-batch'] }),
    ]);
  }, [queryClient]);

  const requestWorkflowVisibility = useCallback(async (
    pending: PendingWorkflowCreation,
    { interactive = false }: { interactive?: boolean } = {},
  ) => {
    const access = pending.scope === 'sandbox'
      ? workflowOperatorAccessRef.current.sandbox
      : workflowOperatorAccessRef.current.instance;

    let token = access.getCachedToken();
    if (!token && interactive) {
      token = await access.authenticate();
    }

    if (!token) {
      return {
        status: 'auth-required' as const,
        message: `Connect to the ${getOperatorLabel(pending.scope).toLowerCase()} to verify that the workflow is visible.`,
      };
    }

    const request = async (authToken: string) =>
      fetch(`${pending.operatorUrl}/api/workflows/${pending.workflowId}`, {
        headers: {
          Authorization: `Bearer ${authToken}`,
        },
      });

    let response = await request(token);

    if (response.status === 401) {
      const refreshedToken = await access.getToken(true);
      if (!refreshedToken) {
        return {
          status: 'auth-required' as const,
          message: `Connect to the ${getOperatorLabel(pending.scope).toLowerCase()} to verify that the workflow is visible.`,
        };
      }
      response = await request(refreshedToken);
    }

    if (response.status === 404) {
      return { status: 'not-found' as const };
    }

    if (response.status === 401 || response.status === 403) {
      return {
        status: 'auth-required' as const,
        message: `Connect to the ${getOperatorLabel(pending.scope).toLowerCase()} to verify that the workflow is visible.`,
      };
    }

    if (!response.ok) {
      const message = await response.text();
      return {
        status: 'error' as const,
        message: message || `Workflow request failed (${response.status})`,
      };
    }

    return { status: 'visible' as const };
  }, []);

  const resolvePendingWorkflow = useCallback(async (
    pending: PendingWorkflowCreation,
    { interactive = false }: { interactive?: boolean } = {},
  ) => {
    if (resolvingPendingKeysRef.current.has(pending.key)) {
      return false;
    }

    if (
      !interactive
      && pending.status === 'timed-out'
      && Date.now() - pending.submittedAt >= WORKFLOW_VISIBILITY_TIMEOUT_MS
    ) {
      return false;
    }

    resolvingPendingKeysRef.current.add(pending.key);
    setResolvingPendingKeys((current) => ({ ...current, [pending.key]: true }));

    try {
      const visibility = await requestWorkflowVisibility(pending, { interactive });

      if (visibility.status === 'visible') {
        removePendingWorkflow(pending.key);
        await invalidateWorkflowQueries();
        if (interactive) {
          toast.success(`Workflow #${pending.workflowId} is now live`);
        }
        return true;
      }

      if (visibility.status === 'auth-required') {
        if (
          pending.status !== 'awaiting-auth'
          || pending.statusMessage !== visibility.message
        ) {
          updatePendingWorkflow(pending.key, {
            status: 'awaiting-auth',
            statusMessage: visibility.message,
          });
        }
        return false;
      }

      const timedOut = Date.now() - pending.submittedAt >= WORKFLOW_VISIBILITY_TIMEOUT_MS;
      if (timedOut) {
        const statusMessage = 'Creation is still processing. Use Check Status to try again.';
        if (
          pending.status !== 'timed-out'
          || pending.statusMessage !== statusMessage
        ) {
          updatePendingWorkflow(pending.key, {
            status: 'timed-out',
            statusMessage,
          });
        }
        return false;
      }

      const statusMessage = visibility.status === 'error'
        ? 'Transaction confirmed. The operator has not exposed the workflow yet, so we will keep checking.'
        : 'Transaction confirmed. Waiting for the operator to publish the workflow.';
      if (
        pending.status !== 'processing'
        || pending.statusMessage !== statusMessage
      ) {
        updatePendingWorkflow(pending.key, {
          status: 'processing',
          statusMessage,
        });
      }
      return false;
    } finally {
      resolvingPendingKeysRef.current.delete(pending.key);
      setResolvingPendingKeys((current) => {
        const next = { ...current };
        delete next[pending.key];
        return next;
      });
    }
  }, [invalidateWorkflowQueries, requestWorkflowVisibility]);

  useEffect(() => {
    if (!address || ownedPendingWorkflows.length === 0) return;

    const tick = () => {
      for (const pending of ownedPendingWorkflows) {
        if (remoteWorkflowKeys.has(getWorkflowIdentityKey(pending.scope, pending.workflowId))) {
          continue;
        }

        void resolvePendingWorkflow(pending);
      }
    };

    tick();
    const intervalId = window.setInterval(tick, WORKFLOW_VISIBILITY_POLL_INTERVAL_MS);
    return () => window.clearInterval(intervalId);
  }, [address, ownedPendingWorkflows, remoteWorkflowKeys, resolvePendingWorkflow]);

  const handleWorkflowAction = useCallback(async (
    workflow: RemoteWorkflowRecord,
    action: 'trigger' | 'cancel',
  ) => {
    if (!workflow.data.targetServiceId) return;
    const jobId = action === 'trigger' ? JOB_IDS.WORKFLOW_TRIGGER : JOB_IDS.WORKFLOW_CANCEL;
    const job = getJobById(workflow.blueprintId, jobId);
    if (!job) return;

    try {
      const hash = await submitJob({
        serviceId: BigInt(workflow.data.targetServiceId),
        jobId,
        args: encodeJobArgs(job, { workflowId: workflow.id }),
        label: `${action === 'trigger' ? 'Trigger' : 'Cancel'} Workflow #${workflow.id}`,
        value: jobValue(jobId),
      });
      if (!hash) return;

      await invalidateWorkflowQueries();
      toast.success(action === 'trigger' ? 'Workflow triggered' : 'Workflow cancelled');
    } catch (e) {
      toast.error(`Failed to ${action} workflow`);
    }
  }, [invalidateWorkflowQueries, submitJob]);

  const handleResolvePendingWorkflow = useCallback(async (pending: PendingWorkflowCreation) => {
    await resolvePendingWorkflow(pending, { interactive: true });
  }, [resolvePendingWorkflow]);

  return (
    <ConsolePage
      title="Automation"
      eyebrow={address ? `${workflows.length} workflow${workflows.length === 1 ? '' : 's'}` : 'Wallet scoped'}
      actions={address ? (
        <Link
          to="/workflows/create"
          className="inline-flex h-10 items-center justify-center gap-2 rounded-[5px] border border-[var(--sandbox-console-brand-border)] bg-[var(--sandbox-console-brand-soft)] px-3 font-display text-sm font-bold text-[var(--sandbox-console-text)] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] transition-[background-color,border-color,box-shadow,transform] duration-150 hover:border-[var(--sandbox-console-brand)] hover:bg-[rgba(142,89,255,0.24)] hover:shadow-[var(--sandbox-console-control-shadow-hover)] active:scale-[0.98]"
        >
          <span className="i-ph:plus-bold text-sm" />
          New Workflow
        </Link>
      ) : (
        <button
          type="button"
          disabled
          className="inline-flex h-10 cursor-not-allowed items-center justify-center gap-2 rounded-[5px] border border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] px-3 font-display text-sm font-bold text-[var(--sandbox-console-muted)] opacity-70"
        >
          <span className="i-ph:plus-bold text-sm" />
          New Workflow
        </button>
      )}
    >
      <div className="space-y-4">
        <ConsoleMetricStrip metrics={workflowMetrics} />

        {address && operatorAuthPrompts.length > 0 ? (
          <ConsoleSection title="Operator Access">
            <div className="divide-y divide-[var(--sandbox-console-border)]">
              {operatorAuthPrompts.map(({ key, label, query }) => (
                <div key={key} className="flex flex-wrap items-center justify-between gap-4 px-3.5 py-3 transition-colors hover:bg-[var(--sandbox-console-hover)]">
                  <div className="min-w-0">
                    <p className="font-display text-sm font-bold text-[var(--sandbox-console-text)]">
                      Connect {label}
                    </p>
                    <p className="mt-0.5 text-xs text-[var(--sandbox-console-muted)]">
                      Sign once to load workflows this wallet can access.
                    </p>
                    {query.authError ? (
                      <p className="mt-1 text-xs text-[var(--sandbox-console-danger)]">{query.authError}</p>
                    ) : null}
                  </div>
                  <WorkflowActionButton
                    tone="secondary"
                    disabled={query.isAuthenticating}
                    onClick={() => {
                      void query.authenticate().then((token) => {
                        if (token) {
                          toast.success(`Connected to ${label.toLowerCase()}`);
                          void query.refetch();
                        }
                      }).catch(() => {
                        toast.error(`Failed to connect to ${label.toLowerCase()}`);
                      });
                    }}
                  >
                    {query.isAuthenticating ? 'Signing...' : `Connect ${label}`}
                  </WorkflowActionButton>
                </div>
              ))}
            </div>
          </ConsoleSection>
      ) : null}

      {operatorErrors.length > 0 ? (
        <ConsoleSection title="Operator Issues">
          <div className="divide-y divide-[var(--sandbox-console-border)]">
            {operatorErrors.map((entry) => (
              <div key={entry.key} className="px-3.5 py-3">
                <p className="font-display text-sm font-bold text-[var(--sandbox-console-danger)]">
                  {entry.label} error
                </p>
                <p className="mt-1 text-xs text-[var(--sandbox-console-muted)]">{entry.message}</p>
              </div>
            ))}
          </div>
        </ConsoleSection>
      ) : null}

        <ConsoleSection title="Workflow Directory">
          {workflows.length > 0 ? (
            <WorkflowTable
              workflows={workflows}
              onTrigger={(workflow) => void handleWorkflowAction(workflow, 'trigger')}
              onCancel={(workflow) => void handleWorkflowAction(workflow, 'cancel')}
              onResolvePending={(pending) => void handleResolvePendingWorkflow(pending)}
              resolvingPendingKeys={resolvingPendingKeys}
              txPending={txStatus === 'pending' || txStatus === 'signing'}
            />
          ) : (
            <EmptyConsoleState
              icon="i-ph:flow-arrow"
              title={!address
                ? 'Connect your wallet to view workflows'
                : isLoading
                  ? 'Loading workflows'
                  : operatorAuthPrompts.length > 0
                    ? 'Authenticate with your operator'
                    : 'No workflows configured'}
              detail={!address
                ? 'Workflow visibility is owner-scoped, so this directory stays empty until the owner wallet is connected.'
                : operatorAuthPrompts.length > 0
                  ? 'Owned workflows are loaded from operator APIs after wallet-scoped authentication.'
                  : 'Create a workflow from a running sandbox or instance to automate recurring tasks.'}
            />
          )}
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}

function WorkflowTable({
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

function WorkflowActionButton({
  children,
  disabled,
  icon,
  onClick,
  tone = 'secondary',
}: {
  children: ReactNode;
  disabled?: boolean;
  icon?: string;
  onClick?: () => void;
  tone?: 'secondary' | 'success';
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'inline-flex h-8 items-center justify-center gap-1.5 rounded-[4px] border px-2.5 font-display text-xs font-bold transition-[background-color,border-color,box-shadow,color,transform] duration-150 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-55',
        tone === 'success'
          ? 'border-[var(--sandbox-console-success-border)] bg-[var(--sandbox-console-success-soft)] text-[var(--sandbox-console-success)] hover:border-[var(--sandbox-console-success)] hover:bg-[color-mix(in_srgb,var(--sandbox-console-success)_18%,transparent)] hover:shadow-[inset_3px_0_0_var(--sandbox-console-success)]'
          : 'border-[var(--sandbox-console-border)] bg-[var(--sandbox-console-control)] text-[var(--sandbox-console-secondary)] hover:border-[var(--sandbox-console-border-hover)] hover:bg-[var(--sandbox-console-control-hover)] hover:text-[var(--sandbox-console-text)] hover:shadow-[var(--sandbox-console-control-shadow-hover)]',
      )}
    >
      {icon ? <span className={cn('text-xs', icon)} /> : null}
      {children}
    </button>
  );
}

function workflowStatusTone(variant: 'stopped' | 'running' | 'secondary' | 'accent') {
  if (variant === 'running') return 'ready';
  if (variant === 'accent') return 'brand';
  if (variant === 'stopped') return 'warn';
  return 'muted';
}

function formatWorkflowDate(value: number | null | undefined, unit: 's' | 'ms') {
  if (value == null || value <= 0) return '--';
  const timestampMs = unit === 's' ? value * 1000 : value;
  return new Date(timestampMs).toLocaleString();
}
