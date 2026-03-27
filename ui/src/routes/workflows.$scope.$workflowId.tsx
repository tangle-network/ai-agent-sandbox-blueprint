import { useCallback, useMemo } from 'react';
import { Link, useParams } from 'react-router';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { useQueryClient } from '@tanstack/react-query';
import { useAccount } from 'wagmi';
import {
  AnimatedPage,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Button,
  Badge,
} from '@tangle-network/blueprint-ui/components';
import { getAddresses, encodeJobArgs, getJobById, useSubmitJob } from '@tangle-network/blueprint-ui';

import { type SandboxAddresses, isContractDeployed } from '~/lib/contracts/chains';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { useWorkflowDetail } from '~/lib/hooks/useWorkflowRuntimeStatus';
import { instanceListStore } from '~/lib/stores/instances';
import { sandboxListStore } from '~/lib/stores/sandboxes';
import {
  getWorkflowBlueprintIdForScope,
  resolveWorkflowTargetLabelFromValues,
  type WorkflowBlueprintId,
  type WorkflowScope,
} from '~/lib/workflows';
import type { Address } from 'viem';

function parseScope(value: string | undefined): WorkflowScope | null {
  if (value === 'sandbox' || value === 'instance' || value === 'tee') return value;
  return null;
}

function parseWorkflowId(value: string | undefined): bigint | null {
  if (!value) return null;
  try {
    return BigInt(value);
  } catch {
    return null;
  }
}

function getContractAddressForScope(
  addrs: SandboxAddresses,
  scope: WorkflowScope,
): Address | undefined {
  const contractAddress = (() => {
    switch (scope) {
      case 'sandbox':
        return addrs.sandboxBlueprint;
      case 'instance':
        return addrs.instanceBlueprint;
      case 'tee':
        return addrs.teeInstanceBlueprint;
    }
  })();

  return isContractDeployed(contractAddress) ? contractAddress : undefined;
}

function formatTimestamp(timestamp: number | null | undefined) {
  if (!timestamp || timestamp <= 0) return 'Not available';
  return new Date(timestamp * 1000).toLocaleString();
}

function formatJson(value: string) {
  if (!value.trim()) return 'Not set';

  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}

function getWorkflowStatusPresentation(
  workflow: { active: boolean; runnable: boolean; targetStatus: 'available' | 'missing' },
) {
  if (!workflow.runnable) {
    return {
      label: 'Not Runnable',
      variant: 'stopped' as const,
      description: workflow.targetStatus === 'missing'
        ? 'This workflow cannot run because its target sandbox or instance is no longer available.'
        : 'This workflow is currently blocked from execution.',
    };
  }

  if (workflow.active) {
    return {
      label: 'Active',
      variant: 'running' as const,
      description: 'This workflow is enabled and can execute normally.',
    };
  }

  return {
    label: 'Inactive',
    variant: 'secondary' as const,
    description: 'This workflow is disabled until it is re-enabled.',
  };
}

function JsonPanel({
  title,
  description,
  value,
}: {
  title: string;
  description: string;
  value: string;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>
        <pre className="overflow-x-auto rounded-lg border border-cloud-elements-dividerColor/30 bg-cloud-elements-background-depth-2 p-4 text-xs font-data text-cloud-elements-textSecondary whitespace-pre-wrap">
          {formatJson(value)}
        </pre>
      </CardContent>
    </Card>
  );
}

export default function WorkflowDetail() {
  const { scope: rawScope, workflowId: rawWorkflowId } = useParams<{
    scope: string;
    workflowId: string;
  }>();
  const scope = parseScope(rawScope);
  const workflowId = parseWorkflowId(rawWorkflowId);
  const workflowIdValue = workflowId !== null ? workflowId.toString() : null;
  const { address } = useAccount();
  const addrs = getAddresses<SandboxAddresses>();
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
  const queryClient = useQueryClient();
  const { submitJob, status: txStatus } = useSubmitJob();

  const blueprintId = useMemo<WorkflowBlueprintId | null>(
    () => (scope ? getWorkflowBlueprintIdForScope(scope) : null),
    [scope],
  );
  const contractAddress = useMemo(() => (scope ? getContractAddressForScope(addrs, scope) : undefined), [addrs, scope]);
  const workflowDetailQuery = useWorkflowDetail(
    scope ?? 'sandbox',
    workflowIdValue,
  );

  const target = useMemo(() => {
    if (!workflowDetailQuery.data || !blueprintId) return null;
    return resolveWorkflowTargetLabelFromValues(
      workflowDetailQuery.data.targetKind,
      workflowDetailQuery.data.targetSandboxId,
      workflowDetailQuery.data.targetServiceId,
      blueprintId,
      sandboxes,
      instances,
    );
  }, [blueprintId, instances, sandboxes, workflowDetailQuery.data]);

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

  const handleWorkflowAction = useCallback(async (
    action: 'trigger' | 'cancel',
  ) => {
    const data = workflowDetailQuery.data;
    if (!data?.targetServiceId || !blueprintId || workflowId === null) return;
    const jobId = action === 'trigger' ? JOB_IDS.WORKFLOW_TRIGGER : JOB_IDS.WORKFLOW_CANCEL;
    const job = getJobById(blueprintId, jobId);
    if (!job) return;

    try {
      const hash = await submitJob({
        serviceId: BigInt(data.targetServiceId),
        jobId,
        args: encodeJobArgs(job, { workflowId }),
        label: `${action === 'trigger' ? 'Trigger' : 'Cancel'} Workflow #${String(workflowId)}`,
        value: jobValue(jobId),
      });
      if (!hash) return;

      await invalidateWorkflowQueries();
      toast.success(action === 'trigger' ? 'Workflow triggered' : 'Workflow cancelled');
    } catch (e) {
      toast.error(`Failed to ${action} workflow`);
    }
  }, [blueprintId, invalidateWorkflowQueries, submitJob, workflowDetailQuery.data, workflowId]);

  const txPending = txStatus === 'pending' || txStatus === 'signing';

  if (!scope || workflowId === null) {
    return (
      <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center">
            <p className="text-cloud-elements-textSecondary">Invalid workflow route</p>
            <Link to="/workflows" className="inline-block mt-4">
              <Button variant="outline" size="sm">Back to Workflows</Button>
            </Link>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  if (!contractAddress) {
    return (
      <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center">
            <p className="text-cloud-elements-textSecondary">Workflow contract is not deployed on the selected network.</p>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  if (!address) {
    return (
      <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center">
            <p className="text-cloud-elements-textSecondary">
              Connect the wallet that owns this workflow to view its details.
            </p>
            <Link to="/workflows" className="inline-block mt-4">
              <Button variant="outline" size="sm">Back to Workflows</Button>
            </Link>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  if (workflowDetailQuery.authRequired) {
    return (
      <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center">
            <p className="text-cloud-elements-textSecondary">
              Authenticate with the operator to load this workflow.
            </p>
            <Button
              variant="secondary"
              size="sm"
              className="mt-4"
              disabled={workflowDetailQuery.isAuthenticating}
              onClick={() => {
                void workflowDetailQuery.authenticate().then((token) => {
                  if (token) {
                    toast.success('Connected to operator');
                    void workflowDetailQuery.refetch();
                  }
                }).catch(() => {
                  toast.error('Failed to connect to operator');
                });
              }}
            >
              {workflowDetailQuery.isAuthenticating ? 'Signing...' : 'Connect Operator'}
            </Button>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  if (workflowDetailQuery.isLoading) {
    return (
      <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center text-cloud-elements-textSecondary">
            Loading workflow...
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  if (workflowDetailQuery.error || !workflowDetailQuery.data || !blueprintId) {
    return (
      <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center">
            <p className="text-cloud-elements-textSecondary">
              {workflowDetailQuery.error?.message || 'Workflow not found'}
            </p>
            <Link to="/workflows" className="inline-block mt-4">
              <Button variant="outline" size="sm">Back to Workflows</Button>
            </Link>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  const workflow = workflowDetailQuery.data;
  const latestExecution = workflow.latestExecution ?? null;
  const lastRunAt = workflow.lastRunAt;
  const status = getWorkflowStatusPresentation(workflow);

  return (
    <AnimatedPage className="mx-auto max-w-5xl px-4 sm:px-6 py-8">
      <div className="flex items-center gap-2 mb-6 text-sm text-cloud-elements-textTertiary">
        <Link to="/workflows" className="hover:text-cloud-elements-textSecondary transition-colors">Workflows</Link>
        <span>/</span>
        <span className="text-cloud-elements-textPrimary font-display">Workflow #{String(workflowId)}</span>
      </div>

      <div className="flex items-start justify-between gap-4 mb-6">
        <div>
          <div className="flex items-center gap-2 flex-wrap">
            <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">
              {workflow.name || `Workflow #${String(workflowId)}`}
            </h1>
            <Badge variant={status.variant}>
              {status.label}
            </Badge>
            {workflow.running ? <Badge variant="accent">Running</Badge> : null}
            <Badge variant="secondary">{target?.kindLabel ?? 'Workflow'}</Badge>
          </div>
          <p className="text-sm text-cloud-elements-textSecondary mt-2">
            {workflow.targetStatus === 'missing'
              ? `Configured for ${target?.label ?? 'this workflow target'}, but the target is no longer available.`
              : target
                ? `Runs on ${target.label}`
                : 'Resolving workflow target...'}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {workflow.active && workflow.targetServiceId !== 0 ? (
            <Button
              variant="success"
              size="sm"
              onClick={() => void handleWorkflowAction('trigger')}
              disabled={txPending || !workflow.runnable}
            >
              <div className="i-ph:play text-xs" />
              Trigger
            </Button>
          ) : null}
          {workflow.active && workflow.targetServiceId !== 0 ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void handleWorkflowAction('cancel')}
              disabled={txPending}
            >
              <div className="i-ph:stop text-xs" />
              Cancel
            </Button>
          ) : null}
        </div>
      </div>

      {!workflow.runnable ? (
        <Card className="mb-6">
          <CardContent className="p-5">
            <div className="flex items-start gap-3">
              <div className="i-ph:warning text-lg text-amber-400 mt-0.5" />
              <div>
                <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                  {status.label}
                </p>
                <p className="text-sm text-cloud-elements-textSecondary mt-1">
                  {status.description}
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
      ) : null}

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mb-6">
        <Card>
          <CardHeader>
            <CardTitle>Workflow Overview</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <div className="flex items-center justify-between gap-4">
              <span className="text-cloud-elements-textTertiary">Workflow ID</span>
              <span className="font-data text-cloud-elements-textPrimary">{String(workflowId)}</span>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-cloud-elements-textTertiary">Trigger</span>
              <span className="text-cloud-elements-textPrimary">{workflow.triggerType}</span>
            </div>
            {workflow.triggerType === 'cron' && (
              <div className="flex items-center justify-between gap-4">
                <span className="text-cloud-elements-textTertiary">Cron Expression</span>
                <span className="font-data text-cloud-elements-textPrimary">{workflow.triggerConfig || 'Not set'}</span>
              </div>
            )}
            <div className="flex items-center justify-between gap-4">
              <span className="text-cloud-elements-textTertiary">Scope</span>
              <span className="text-cloud-elements-textPrimary capitalize">{workflow.scope}</span>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-cloud-elements-textTertiary">Status</span>
              <span className="text-cloud-elements-textPrimary">{status.label}</span>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-cloud-elements-textTertiary">Target Service</span>
              <span className="text-cloud-elements-textPrimary">{workflow.targetServiceId}</span>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-cloud-elements-textTertiary">Target Sandbox</span>
              <span className="text-right text-cloud-elements-textPrimary">
                {workflow.targetSandboxId || 'Not set'}
              </span>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Execution Status</CardTitle>
            <CardDescription>
              Live operator visibility for this workflow&apos;s latest execution.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            {!workflow ? (
              <div className="rounded-lg border border-cloud-elements-dividerColor/40 bg-cloud-elements-background-depth-2 p-4">
                <p className="text-cloud-elements-textSecondary">
                  This operator does not currently have local runtime status for the selected workflow.
                </p>
              </div>
            ) : (
              <>
                <div className="flex items-center justify-between gap-4">
                  <span className="text-cloud-elements-textTertiary">Running</span>
                  <span className="text-cloud-elements-textPrimary">{workflow.running ? 'Yes' : 'No'}</span>
                </div>
                <div className="flex items-center justify-between gap-4">
                  <span className="text-cloud-elements-textTertiary">Last Run</span>
                  <span className="text-cloud-elements-textPrimary">{formatTimestamp(lastRunAt)}</span>
                </div>
                <div className="flex items-center justify-between gap-4">
                  <span className="text-cloud-elements-textTertiary">Next Scheduled Run</span>
                  <span className="text-cloud-elements-textPrimary">
                    {workflow.runnable ? formatTimestamp(workflow.nextRunAt) : 'Not Runnable'}
                  </span>
                </div>
                <div className="flex items-center justify-between gap-4">
                  <span className="text-cloud-elements-textTertiary">Latest Result</span>
                  <span className="text-cloud-elements-textPrimary">
                    {latestExecution
                      ? (latestExecution.success ? 'Success' : 'Failed')
                      : 'No executions recorded'}
                  </span>
                </div>
                {latestExecution ? (
                  <>
                    <div className="flex items-center justify-between gap-4">
                      <span className="text-cloud-elements-textTertiary">Duration</span>
                      <span className="text-cloud-elements-textPrimary">{latestExecution.durationMs} ms</span>
                    </div>
                    <div className="flex items-center justify-between gap-4">
                      <span className="text-cloud-elements-textTertiary">Token Usage</span>
                      <span className="text-cloud-elements-textPrimary">
                        {latestExecution.inputTokens} in / {latestExecution.outputTokens} out
                      </span>
                    </div>
                    <div className="flex items-center justify-between gap-4">
                      <span className="text-cloud-elements-textTertiary">Trace ID</span>
                      <span className="font-data text-cloud-elements-textPrimary break-all text-right">{latestExecution.traceId || 'Not available'}</span>
                    </div>
                    <div className="flex items-center justify-between gap-4">
                      <span className="text-cloud-elements-textTertiary">Session ID</span>
                      <span className="font-data text-cloud-elements-textPrimary break-all text-right">{latestExecution.sessionId || 'Not available'}</span>
                    </div>
                  </>
                ) : null}
              </>
            )}
          </CardContent>
        </Card>
      </div>

      {latestExecution ? (
        <div className="grid grid-cols-1 gap-6 mb-6">
          <Card>
            <CardHeader>
              <CardTitle>Latest Execution Output</CardTitle>
              <CardDescription>
                Captured from the operator&apos;s most recent workflow execution.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <pre className="overflow-x-auto rounded-lg border border-cloud-elements-dividerColor/30 bg-cloud-elements-background-depth-2 p-4 text-xs font-data text-cloud-elements-textSecondary whitespace-pre-wrap">
                {latestExecution.result || 'No result output'}
              </pre>
              {!latestExecution.success && latestExecution.error ? (
                <div>
                  <p className="text-sm font-display font-medium text-rose-300 mb-2">Execution Error</p>
                  <pre className="overflow-x-auto rounded-lg border border-rose-500/20 bg-rose-500/5 p-4 text-xs font-data text-rose-200 whitespace-pre-wrap">
                    {latestExecution.error}
                  </pre>
                </div>
              ) : null}
            </CardContent>
          </Card>
        </div>
      ) : null}

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        <JsonPanel
          title="Workflow Definition"
          description="The stored task payload that the workflow executes."
          value={workflow.workflowJson}
        />
        <JsonPanel
          title="Execution Config"
          description="Additional runtime configuration saved with the workflow."
          value={workflow.sandboxConfigJson}
        />
      </div>
    </AnimatedPage>
  );
}
