import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import { useStore } from '@nanostores/react';
import { useQueryClient } from '@tanstack/react-query';
import { useAccount } from 'wagmi';
import { AnimatedPage, StaggerContainer, StaggerItem } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Select } from '@tangle-network/blueprint-ui/components';
import { useWorkflowSummaries, type WorkflowOperatorSummary } from '~/lib/hooks/useWorkflowRuntimeStatus';
import { getAddresses, publicClient, tangleJobsAbi, useSubmitJob } from '@tangle-network/blueprint-ui';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { getJobById } from '@tangle-network/blueprint-ui';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { cn } from '@tangle-network/blueprint-ui';
import { decodeEventLog, type Address } from 'viem';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { sandboxListStore } from '~/lib/stores/sandboxes';
import { instanceListStore } from '~/lib/stores/instances';
import {
  WORKFLOW_TARGET_INSTANCE,
  WORKFLOW_TARGET_SANDBOX,
  buildWorkflowDetailPath,
  getWorkflowBlueprintIdForScope,
  resolveWorkflowTargetLabelFromValues,
  type WorkflowBlueprintId,
  type WorkflowScope,
} from '~/lib/workflows';

type WorkflowTarget = {
  key: string;
  value: string;
  label: string;
  kindLabel: string;
  description: string;
  serviceId: string;
  targetKind: number;
  targetSandboxId: string;
  blueprintId: WorkflowBlueprintId;
  contractAddress: Address;
};

type WorkflowRecord = {
  id: bigint;
  scope: WorkflowScope;
  blueprintId: WorkflowBlueprintId;
  data: WorkflowOperatorSummary;
  targetLabel: string;
  kindLabel: string;
};

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

export default function Workflows() {
  const queryClient = useQueryClient();
  const [searchParams] = useSearchParams();
  const { address } = useAccount();
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
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

  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState('');
  const [selectedTargetKey, setSelectedTargetKey] = useState('');
  const [triggerType, setTriggerType] = useState('cron');
  const [triggerConfig, setTriggerConfig] = useState('');
  const [workflowJson, setWorkflowJson] = useState('{\n  "prompt": ""\n}');
  const [sandboxConfigJson, setSandboxConfigJson] = useState('{}');
  const [createError, setCreateError] = useState<string | null>(null);
  const [isVerifyingCreate, setIsVerifyingCreate] = useState(false);

  const availableTargets = useMemo<WorkflowTarget[]>(() => {
    const sandboxTargets: WorkflowTarget[] = sandboxes
      .filter((sandbox) => sandbox.status === 'running' && !!sandbox.sandboxId && !!sandbox.serviceId)
      .map((sandbox) => ({
        key: `sandbox:${sandbox.sandboxId ?? sandbox.localId}`,
        value: `sandbox:${sandbox.sandboxId ?? sandbox.localId}`,
        label: sandbox.name,
        kindLabel: 'Sandbox',
        description: sandbox.image,
        serviceId: sandbox.serviceId,
        targetKind: WORKFLOW_TARGET_SANDBOX,
        targetSandboxId: sandbox.sandboxId ?? '',
        blueprintId: 'ai-agent-sandbox-blueprint',
        contractAddress: addrs.sandboxBlueprint,
      }));

    const instanceTargets: WorkflowTarget[] = instances
      .filter((instance) => instance.status === 'running' && !!instance.serviceId)
      .map((instance) => {
        const blueprintId: WorkflowBlueprintId = instance.teeEnabled
          ? 'ai-agent-tee-instance-blueprint'
          : 'ai-agent-instance-blueprint';
        return {
          key: `instance:${instance.id}`,
          value: `instance:${instance.id}`,
          label: instance.name,
          kindLabel: instance.teeEnabled ? 'TEE Instance' : 'Instance',
          description: instance.image,
          serviceId: instance.serviceId,
          targetKind: WORKFLOW_TARGET_INSTANCE,
          targetSandboxId: '',
          blueprintId,
          contractAddress: instance.teeEnabled ? addrs.teeInstanceBlueprint : addrs.instanceBlueprint,
        };
      });

    return [...sandboxTargets, ...instanceTargets];
  }, [addrs.instanceBlueprint, addrs.sandboxBlueprint, addrs.teeInstanceBlueprint, instances, sandboxes]);

  useEffect(() => {
    const requestedTarget = searchParams.get('target');
    if (!requestedTarget && availableTargets.length === 0) return;

    const normalizedRequested = requestedTarget ? decodeURIComponent(requestedTarget) : '';
    const targetExists = normalizedRequested
      ? availableTargets.some((target) => target.value === normalizedRequested)
      : false;

    if (targetExists) {
      setSelectedTargetKey(normalizedRequested);
      setShowCreate(true);
      return;
    }

    if (!selectedTargetKey && availableTargets.length > 0) {
      setSelectedTargetKey(availableTargets[0].value);
    }
  }, [availableTargets, searchParams, selectedTargetKey]);

  const selectedTarget = useMemo(
    () => availableTargets.find((target) => target.value === selectedTargetKey) ?? null,
    [availableTargets, selectedTargetKey],
  );

  const workflows = useMemo<WorkflowRecord[]>(() => {
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
      deduped.set(`${entry.workflow.scope}:${entry.workflow.workflowId}`, entry);
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
          id: BigInt(workflow.workflowId),
          scope: workflow.scope,
          blueprintId,
          data: workflow,
          targetLabel: resolvedTarget.label,
          kindLabel: resolvedTarget.kindLabel,
        };
      })
      .filter((workflow): workflow is WorkflowRecord => workflow !== null)
      .sort((left, right) => {
        const leftUpdated = left.data.lastRunAt
          ?? left.data.latestExecution?.executedAt
          ?? left.data.nextRunAt
          ?? 0;
        const rightUpdated = right.data.lastRunAt
          ?? right.data.latestExecution?.executedAt
          ?? right.data.nextRunAt
          ?? 0;
        return rightUpdated - leftUpdated || Number(right.id - left.id);
      });
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
      sandboxWorkflowSummaries.error ? { key: `error:${sandboxOperatorUrl}`, label: 'Sandbox operator', message: sandboxWorkflowSummaries.error.message } : null,
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

  const waitForWorkflowOnChain = useCallback(async (contractAddress: Address, workflowId: bigint) => {
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const result = await publicClient.readContract({
        address: contractAddress,
        abi: [
          {
            type: 'function',
            name: 'getWorkflow',
            inputs: [{ name: 'workflowId', type: 'uint64' }],
            outputs: [
              {
                name: '',
                type: 'tuple',
                components: [
                  { name: 'name', type: 'string' },
                  { name: 'workflow_json', type: 'string' },
                  { name: 'trigger_type', type: 'string' },
                  { name: 'trigger_config', type: 'string' },
                  { name: 'sandbox_config_json', type: 'string' },
                  { name: 'target_kind', type: 'uint8' },
                  { name: 'target_sandbox_id', type: 'string' },
                  { name: 'target_service_id', type: 'uint64' },
                  { name: 'active', type: 'bool' },
                  { name: 'created_at', type: 'uint64' },
                  { name: 'updated_at', type: 'uint64' },
                  { name: 'last_triggered_at', type: 'uint64' },
                ],
              },
            ],
            stateMutability: 'view',
          },
        ],
        functionName: 'getWorkflow',
        args: [workflowId],
      }) as { name?: string };

      if (result.name?.trim()) {
        return true;
      }

      await new Promise((resolve) => window.setTimeout(resolve, 1500));
    }

    return false;
  }, []);

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

  const handleCreate = useCallback(async () => {
    if (!name || !selectedTarget) return;
    const job = getJobById(selectedTarget.blueprintId, JOB_IDS.WORKFLOW_CREATE);
    if (!job) return;
    setCreateError(null);

    // Validate cron expression before submitting the on-chain transaction.
    // The Rust cron crate requires 6 or 7 fields (sec min hour dom mon dow [year]).
    if (triggerType === 'cron' && triggerConfig.trim()) {
      const fields = triggerConfig.trim().split(/\s+/);
      if (fields.length < 6 || fields.length > 7) {
        setCreateError(
          `Cron expression must have 6 or 7 fields (sec min hour dom mon dow [year]), got ${fields.length}. Example: 0 */5 * * * *`,
        );
        return;
      }
    }

    setIsVerifyingCreate(true);

    try {
      const hash = await submitJob({
        serviceId: BigInt(selectedTarget.serviceId),
        jobId: JOB_IDS.WORKFLOW_CREATE,
        args: encodeJobArgs(job, {
          name,
          workflowJson,
          triggerType,
          triggerConfig,
          sandboxConfigJson,
          targetKind: selectedTarget.targetKind,
          targetSandboxId: selectedTarget.targetSandboxId,
          targetServiceId: Number(selectedTarget.serviceId),
        }),
        label: `Create Workflow: ${name}`,
        value: jobValue(JOB_IDS.WORKFLOW_CREATE),
      });

      if (!hash) {
        return;
      }

      const receipt = await publicClient.waitForTransactionReceipt({ hash });
      let workflowCallId: bigint | null = null;

      for (const log of receipt.logs) {
        try {
          const decoded = decodeEventLog({
            abi: tangleJobsAbi,
            data: log.data,
            topics: log.topics,
          });
          if (decoded.eventName === 'JobSubmitted' && 'callId' in decoded.args) {
            workflowCallId = decoded.args.callId as bigint;
            break;
          }
        } catch {
          // Ignore unrelated logs.
        }
      }

      if (workflowCallId === null) {
        throw new Error('Transaction confirmed, but the workflow call ID could not be found.');
      }

      const visible = await waitForWorkflowOnChain(selectedTarget.contractAddress, workflowCallId);
      if (!visible) {
        throw new Error('Transaction confirmed, but the workflow was not readable from the chain.');
      }

      await invalidateWorkflowQueries();

      setShowCreate(false);
      setName('');
      setTriggerConfig('');
      setWorkflowJson('{\n  "prompt": ""\n}');
      setSandboxConfigJson('{}');
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Workflow creation failed';
      setCreateError(message);
    } finally {
      setIsVerifyingCreate(false);
    }
  }, [
    invalidateWorkflowQueries,
    name,
    sandboxConfigJson,
    selectedTarget,
    submitJob,
    triggerConfig,
    triggerType,
    waitForWorkflowOnChain,
    workflowJson,
  ]);

  const handleWorkflowAction = useCallback(async (
    workflow: WorkflowRecord,
    action: 'trigger' | 'cancel',
  ) => {
    if (!workflow.data.targetServiceId) return;
    const jobId = action === 'trigger' ? JOB_IDS.WORKFLOW_TRIGGER : JOB_IDS.WORKFLOW_CANCEL;
    const job = getJobById(workflow.blueprintId, jobId);
    if (!job) return;

    await submitJob({
      serviceId: BigInt(workflow.data.targetServiceId),
      jobId,
      args: encodeJobArgs(job, { workflowId: workflow.id }),
      label: `${action === 'trigger' ? 'Trigger' : 'Cancel'} Workflow #${workflow.id}`,
      value: jobValue(jobId),
    });

    await invalidateWorkflowQueries();
  }, [invalidateWorkflowQueries, submitJob]);

  const triggerOptions = [
    { label: 'Cron Schedule', value: 'cron' },
    { label: 'Webhook', value: 'webhook' },
    { label: 'Manual', value: 'manual' },
  ];

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
        <Button onClick={() => setShowCreate((current) => !current)} disabled={!address || availableTargets.length === 0}>
          <div className={showCreate ? 'i-ph:x text-base' : 'i-ph:plus text-base'} />
          {showCreate ? 'Cancel' : 'New Workflow'}
        </Button>
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
                        if (token) void query.refetch();
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

      {availableTargets.length === 0 && (
        <Card className="mb-6">
          <CardContent className="p-5">
            <div className="flex items-center gap-3">
              <div className="i-ph:warning text-lg text-amber-400" />
              <div>
                <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                  No runnable targets available
                </p>
                <p className="text-xs text-cloud-elements-textTertiary mt-1">
                  Start a sandbox or instance first. Workflow targets are derived from running resources, not entered as service IDs.
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {showCreate && (
        <Card className="mb-6">
          <CardHeader>
            <CardTitle>Create Workflow</CardTitle>
            <CardDescription>Choose the resource this workflow will automate, then define the trigger and task payload.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Name</label>
                <Input value={name} onChange={(event) => setName(event.target.value)} placeholder="daily-backup" />
              </div>
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Runs On</label>
                <Select
                  value={selectedTargetKey}
                  onValueChange={setSelectedTargetKey}
                  options={availableTargets.map((target) => ({
                    value: target.value,
                    label: `${target.kindLabel}: ${target.label}`,
                  }))}
                />
                {selectedTarget && (
                  <p className="text-[11px] text-cloud-elements-textTertiary mt-1">
                    {selectedTarget.description}
                  </p>
                )}
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Trigger Type</label>
                <Select
                  value={triggerType}
                  onValueChange={setTriggerType}
                  options={triggerOptions}
                />
              </div>
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Trigger Config</label>
                <Input
                  value={triggerConfig}
                  onChange={(event) => setTriggerConfig(event.target.value)}
                  placeholder={triggerType === 'cron' ? '0 */6 * * *' : triggerType === 'webhook' ? 'https://...' : 'Optional'}
                />
              </div>
            </div>

            <div>
              <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Task Definition (JSON)</label>
              <textarea
                value={workflowJson}
                onChange={(event) => setWorkflowJson(event.target.value)}
                placeholder='{"prompt":"Summarize the latest logs"}'
                rows={6}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
              <p className="text-[11px] text-cloud-elements-textTertiary mt-1">
                The selected resource supplies the runtime target automatically. Do not include `sidecar_url` here.
              </p>
            </div>

            <div>
              <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Execution Config (JSON)</label>
              <textarea
                value={sandboxConfigJson}
                onChange={(event) => setSandboxConfigJson(event.target.value)}
                placeholder='{"image":"agent-dev:latest"}'
                rows={3}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
            </div>

            {selectedTarget && (
              <div className="glass-card rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  <Badge variant="accent">{selectedTarget.kindLabel}</Badge>
                  <span className="text-sm font-display font-medium text-cloud-elements-textPrimary">{selectedTarget.label}</span>
                </div>
                <p className="text-xs text-cloud-elements-textTertiary">
                  Job routing will use the target resource automatically. Service #{selectedTarget.serviceId} stays internal.
                </p>
              </div>
            )}

            <div className="flex justify-end">
              <Button onClick={handleCreate} disabled={!name || !selectedTarget || txStatus === 'pending' || isVerifyingCreate}>
                <div className="i-ph:flow-arrow text-sm" />
                {isVerifyingCreate ? 'Verifying Workflow...' : 'Create Workflow'}
              </Button>
            </div>

            {createError ? <p className="text-sm text-rose-400">{createError}</p> : null}
          </CardContent>
        </Card>
      )}

      {workflows.length > 0 ? (
        <StaggerContainer className="space-y-3">
          {workflows.map((workflow) => (
            <StaggerItem key={`${workflow.blueprintId}:${String(workflow.id)}`}>
              <WorkflowCard
                workflow={workflow}
                onTrigger={() => void handleWorkflowAction(workflow, 'trigger')}
                onCancel={() => void handleWorkflowAction(workflow, 'cancel')}
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
  txPending,
}: {
  workflow: WorkflowRecord;
  onTrigger: () => void;
  onCancel: () => void;
  txPending: boolean;
}) {
  const { id, data, targetLabel, kindLabel } = workflow;
  const status = getWorkflowStatusPresentation(data);

  const triggerLabel: Record<string, string> = {
    cron: 'Cron',
    webhook: 'Webhook',
    manual: 'Manual',
  };

  const canTrigger = data.runnable && data.targetServiceId !== 0;
  const canCancel = data.active && data.targetServiceId !== 0;
  const detailPath = buildWorkflowDetailPath(workflow.scope, id);

  return (
    <Card>
      <CardContent className="p-5">
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-4 min-w-0">
            <div className={cn(
              'w-10 h-10 rounded-lg flex items-center justify-center shrink-0',
              data.active ? 'bg-teal-500/10' : 'bg-cloud-elements-background-depth-3',
            )}>
              <div className={cn(
                'i-ph:flow-arrow text-lg',
                data.active ? 'text-teal-400' : 'text-cloud-elements-textTertiary',
              )} />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <Link
                  to={detailPath}
                  className="text-sm font-display font-semibold text-cloud-elements-textPrimary hover:text-violet-400 transition-colors"
                >
                  {data.name || `Workflow #${String(id)}`}
                </Link>
                <Badge variant={status.variant}>
                  {status.label}
                </Badge>
                <Badge variant="accent">
                  {triggerLabel[data.triggerType] ?? data.triggerType}
                </Badge>
                <Badge variant="secondary">{kindLabel}</Badge>
              </div>
              <div className="flex items-center gap-3 mt-1 text-xs text-cloud-elements-textTertiary flex-wrap">
                <span>
                  {data.targetStatus === 'missing'
                    ? `Target missing: ${targetLabel}`
                    : `Runs on ${targetLabel}`}
                </span>
                {data.triggerConfig && (
                  <>
                    <span className="text-cloud-elements-dividerColor">·</span>
                    <span className="font-data">{data.triggerConfig}</span>
                  </>
                )}
                <span className="text-cloud-elements-dividerColor">·</span>
                <span>{status.detail}</span>
                {data.lastRunAt && data.lastRunAt > 0 && (
                  <>
                    <span className="text-cloud-elements-dividerColor">·</span>
                    <span>Last: {new Date(data.lastRunAt * 1000).toLocaleString()}</span>
                  </>
                )}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Link to={detailPath}>
              <Button variant="outline" size="sm">
                View Details
              </Button>
            </Link>
            {data.active && data.targetServiceId !== 0 ? (
              <Button variant="success" size="sm" onClick={onTrigger} disabled={txPending || !canTrigger}>
                <div className="i-ph:play text-xs" />
                Trigger
              </Button>
            ) : null}
            {canCancel ? (
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
