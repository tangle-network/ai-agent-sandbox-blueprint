import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { useQueryClient } from '@tanstack/react-query';
import { useAccount } from 'wagmi';
import { AnimatedPage, StaggerContainer, StaggerItem } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
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
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">Workflows</h1>
          <p className="text-sm text-cloud-elements-textSecondary mt-1">
            {!address
              ? 'Connect your wallet to view workflows you own'
              : workflows.length > 0
                ? `${workflows.length} workflow${workflows.length > 1 ? 's' : ''}`
                : 'Automation across your sandboxes and instances'}
          </p>
        </div>
        {address ? (
          <Link to="/workflows/create">
            <Button>
              <div className="i-ph:plus text-base" />
              New Workflow
            </Button>
          </Link>
        ) : (
          <Button disabled>
            <div className="i-ph:plus text-base" />
            New Workflow
          </Button>
        )}
      </div>

      {address && operatorAuthPrompts.length > 0 ? (
        <div className="space-y-3 mb-6">
          {operatorAuthPrompts.map(({ key, label, query }) => (
            <Card key={key}>
              <CardContent className="p-5">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                  <div>
                    <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                      Connect {label}
                    </p>
                    <p className="text-xs text-cloud-elements-textTertiary mt-1">
                      Sign once to load the workflows this wallet can access on that operator.
                    </p>
                    {query.authError ? (
                      <p className="text-xs text-rose-300 mt-2">{query.authError}</p>
                    ) : null}
                  </div>
                  <Button
                    variant="secondary"
                    size="sm"
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
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : null}

      {operatorErrors.length > 0 ? (
        <div className="space-y-3 mb-6">
          {operatorErrors.map((entry) => (
            <Card key={entry.key}>
              <CardContent className="p-5">
                <p className="text-sm font-display font-medium text-rose-300">
                  {entry.label} error
                </p>
                <p className="text-xs text-rose-200 mt-1">{entry.message}</p>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : null}

      {workflows.length > 0 ? (
        <StaggerContainer className="space-y-3">
          {workflows.map((workflow) => (
            <StaggerItem key={`${workflow.kind}:${workflow.blueprintId}:${String(workflow.id)}`}>
              <WorkflowCard
                workflow={workflow}
                onTrigger={workflow.kind === 'remote'
                  ? () => void handleWorkflowAction(workflow, 'trigger')
                  : undefined}
                onCancel={workflow.kind === 'remote'
                  ? () => void handleWorkflowAction(workflow, 'cancel')
                  : undefined}
                onResolvePending={workflow.kind === 'pending'
                  ? () => void handleResolvePendingWorkflow(workflow.pending)
                  : undefined}
                pendingActionLoading={workflow.kind === 'pending' && !!resolvingPendingKeys[workflow.pending.key]}
                txPending={txStatus === 'pending' || txStatus === 'signing'}
              />
            </StaggerItem>
          ))}
        </StaggerContainer>
      ) : (
        <Card>
          <CardContent className="p-6">
            <div className="py-16 text-center">
              <div className="i-ph:flow-arrow text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-cloud-elements-textSecondary font-display">
                {!address
                  ? 'Connect your wallet to view workflows'
                  : isLoading
                    ? 'Loading workflows...'
                    : operatorAuthPrompts.length > 0
                      ? 'Authenticate with your operator to load workflows'
                      : 'No workflows configured'}
              </p>
              <p className="text-sm text-cloud-elements-textTertiary mt-1">
                {!address
                  ? 'Workflow visibility is owner-scoped, so the list stays empty until you connect the owner wallet'
                  : operatorAuthPrompts.length > 0
                    ? 'Owned workflows now come from the operator API, so you need an operator session before the list can load'
                    : 'Create a workflow from a running sandbox or instance to automate recurring tasks'}
              </p>
            </div>
          </CardContent>
        </Card>
      )}
    </AnimatedPage>
  );
}

function WorkflowCard({
  workflow,
  onTrigger,
  onCancel,
  onResolvePending,
  pendingActionLoading,
  txPending,
}: {
  workflow: WorkflowRecord;
  onTrigger?: () => void;
  onCancel?: () => void;
  onResolvePending?: () => void;
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
  const navigate = useNavigate();

  return (
    <Card
      className={detailPath ? 'cursor-pointer transition-colors hover:bg-cloud-elements-background-depth-2' : undefined}
      onClick={detailPath ? () => navigate(detailPath) : undefined}
    >
      <CardContent className="p-5">
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-4 min-w-0">
            <div className={cn(
              'w-10 h-10 rounded-lg flex items-center justify-center shrink-0',
              isPending
                ? 'bg-amber-500/10'
                : workflow.data.active
                  ? 'bg-teal-500/10'
                  : 'bg-cloud-elements-background-depth-3',
            )}>
              <div className={cn(
                'i-ph:flow-arrow text-lg',
                isPending
                  ? 'text-amber-300'
                  : workflow.data.active
                    ? 'text-teal-400'
                    : 'text-cloud-elements-textTertiary',
              )} />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-sm font-display font-semibold text-cloud-elements-textPrimary">
                  {name || `Workflow #${String(workflow.id)}`}
                </span>
                <Badge variant={status.variant}>
                  {status.label}
                </Badge>
                <Badge variant="accent">
                  {triggerLabel[triggerType] ?? triggerType}
                </Badge>
                <Badge variant="secondary">{workflow.kindLabel}</Badge>
              </div>
              <div className="flex items-center gap-3 mt-1 text-xs text-cloud-elements-textTertiary flex-wrap">
                <span>
                  {isPending
                    ? `Will run on ${workflow.targetLabel}`
                    : workflow.data.targetStatus === 'missing'
                      ? `Target missing: ${workflow.targetLabel}`
                      : `Runs on ${workflow.targetLabel}`}
                </span>
                {triggerConfig ? (
                  <>
                    <span className="text-cloud-elements-dividerColor">·</span>
                    <span className="font-data">{triggerConfig}</span>
                  </>
                ) : null}
                <span className="text-cloud-elements-dividerColor">·</span>
                <span>{status.detail}</span>
                {!isPending && workflow.data.lastRunAt && workflow.data.lastRunAt > 0 ? (
                  <>
                    <span className="text-cloud-elements-dividerColor">·</span>
                    <span>Last: {new Date(workflow.data.lastRunAt * 1000).toLocaleString()}</span>
                  </>
                ) : null}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
            {isPending ? (
              <Button
                variant="secondary"
                size="sm"
                onClick={onResolvePending}
                disabled={pendingActionLoading}
              >
                {pendingActionLoading
                  ? 'Checking...'
                  : workflow.pending.status === 'awaiting-auth'
                    ? 'Connect Operator'
                    : 'Check Status'}
              </Button>
            ) : null}
            {!isPending && workflow.data.active && workflow.data.targetServiceId !== 0 ? (
              <Button variant="success" size="sm" onClick={onTrigger} disabled={txPending || !canTrigger}>
                <div className="i-ph:play text-xs" />
                Trigger
              </Button>
            ) : null}
            {!isPending && canCancel ? (
              <Button variant="secondary" size="sm" onClick={onCancel} disabled={txPending}>
                <div className="i-ph:stop text-xs" />
                Cancel
              </Button>
            ) : null}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
