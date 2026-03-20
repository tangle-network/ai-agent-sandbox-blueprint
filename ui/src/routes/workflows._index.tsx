import { useCallback, useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router';
import { useStore } from '@nanostores/react';
import { useQueryClient } from '@tanstack/react-query';
import { AnimatedPage, StaggerContainer, StaggerItem } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Select } from '@tangle-network/blueprint-ui/components';
import {
  useWorkflowIdsForAddress,
  useWorkflowBatchForAddress,
  type WorkflowView,
} from '~/lib/hooks/useSandboxReads';
import { getAddresses, publicClient, tangleJobsAbi, useSubmitJob } from '@tangle-network/blueprint-ui';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { getJobById } from '@tangle-network/blueprint-ui';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { cn } from '@tangle-network/blueprint-ui';
import { decodeEventLog, type Address } from 'viem';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import { sandboxListStore, type LocalSandbox } from '~/lib/stores/sandboxes';
import { instanceListStore, type LocalInstance } from '~/lib/stores/instances';

type WorkflowBlueprintId =
  | 'ai-agent-sandbox-blueprint'
  | 'ai-agent-instance-blueprint'
  | 'ai-agent-tee-instance-blueprint';

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
  blueprintId: WorkflowBlueprintId;
  contractAddress: Address;
  data: WorkflowView | null;
  targetLabel: string;
  kindLabel: string;
};

type WorkflowSource = {
  blueprintId: WorkflowBlueprintId;
  contractAddress: Address;
};

const WORKFLOW_TARGET_SANDBOX = 0;
const WORKFLOW_TARGET_INSTANCE = 1;

function getWorkflowContractAddress(address: Address): Address | undefined {
  return isContractDeployed(address) ? address : undefined;
}

export default function Workflows() {
  const queryClient = useQueryClient();
  const [searchParams] = useSearchParams();
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
  const { submitJob, status: txStatus } = useSubmitJob();
  const addrs = getAddresses<SandboxAddresses>();

  const sources = useMemo<WorkflowSource[]>(() => ([
    { blueprintId: 'ai-agent-sandbox-blueprint', contractAddress: addrs.sandboxBlueprint },
    { blueprintId: 'ai-agent-instance-blueprint', contractAddress: addrs.instanceBlueprint },
    { blueprintId: 'ai-agent-tee-instance-blueprint', contractAddress: addrs.teeInstanceBlueprint },
  ]), [addrs.instanceBlueprint, addrs.sandboxBlueprint, addrs.teeInstanceBlueprint]);

  const sandboxWorkflowIds = useWorkflowIdsForAddress(getWorkflowContractAddress(addrs.sandboxBlueprint), false);
  const instanceWorkflowIds = useWorkflowIdsForAddress(getWorkflowContractAddress(addrs.instanceBlueprint), false);
  const teeWorkflowIds = useWorkflowIdsForAddress(getWorkflowContractAddress(addrs.teeInstanceBlueprint), false);

  const sandboxWorkflowData = useWorkflowBatchForAddress(
    getWorkflowContractAddress(addrs.sandboxBlueprint),
    (sandboxWorkflowIds.data ?? []) as bigint[],
  );
  const instanceWorkflowData = useWorkflowBatchForAddress(
    getWorkflowContractAddress(addrs.instanceBlueprint),
    (instanceWorkflowIds.data ?? []) as bigint[],
  );
  const teeWorkflowData = useWorkflowBatchForAddress(
    getWorkflowContractAddress(addrs.teeInstanceBlueprint),
    (teeWorkflowIds.data ?? []) as bigint[],
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
    const groups = [
      {
        blueprintId: sources[0].blueprintId,
        contractAddress: sources[0].contractAddress,
        ids: sandboxWorkflowIds.data ?? [],
        data: sandboxWorkflowData.data ?? [],
      },
      {
        blueprintId: sources[1].blueprintId,
        contractAddress: sources[1].contractAddress,
        ids: instanceWorkflowIds.data ?? [],
        data: instanceWorkflowData.data ?? [],
      },
      {
        blueprintId: sources[2].blueprintId,
        contractAddress: sources[2].contractAddress,
        ids: teeWorkflowIds.data ?? [],
        data: teeWorkflowData.data ?? [],
      },
    ];

    return groups
      .flatMap((group) =>
        group.ids.map((id, index) => {
          const batchResult = group.data[index];
          const data = batchResult?.status === 'success' ? batchResult.result : null;
          const resolvedTarget = resolveWorkflowTargetLabel(
            data,
            group.blueprintId,
            sandboxes,
            instances,
          );
          return {
            id: id as bigint,
            blueprintId: group.blueprintId,
            contractAddress: group.contractAddress,
            data,
            targetLabel: resolvedTarget.label,
            kindLabel: resolvedTarget.kindLabel,
          };
        }),
      )
      .sort((left, right) => {
        const leftUpdated = left.data?.updated_at ?? 0;
        const rightUpdated = right.data?.updated_at ?? 0;
        return rightUpdated - leftUpdated;
      });
  }, [
    instanceWorkflowData.data,
    instanceWorkflowIds.data,
    instances,
    sandboxWorkflowData.data,
    sandboxWorkflowIds.data,
    sandboxes,
    sources,
    teeWorkflowData.data,
    teeWorkflowIds.data,
  ]);

  const isLoading =
    sandboxWorkflowIds.isLoading
    || instanceWorkflowIds.isLoading
    || teeWorkflowIds.isLoading;

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
          if (decoded.eventName === 'JobCalled' && 'callId' in decoded.args) {
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
    if (!workflow.data || !workflow.data.target_service_id) return;
    const jobId = action === 'trigger' ? JOB_IDS.WORKFLOW_TRIGGER : JOB_IDS.WORKFLOW_CANCEL;
    const job = getJobById(workflow.blueprintId, jobId);
    if (!job) return;

    await submitJob({
      serviceId: BigInt(workflow.data.target_service_id),
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
            {workflows.length > 0 ? `${workflows.length} workflow${workflows.length > 1 ? 's' : ''}` : 'Automation across your sandboxes and instances'}
          </p>
        </div>
        <Button onClick={() => setShowCreate((current) => !current)} disabled={availableTargets.length === 0}>
          <div className={showCreate ? 'i-ph:x text-base' : 'i-ph:plus text-base'} />
          {showCreate ? 'Cancel' : 'New Workflow'}
        </Button>
      </div>

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
                {isLoading ? 'Loading workflows...' : 'No workflows configured'}
              </p>
              <p className="text-sm text-cloud-elements-textTertiary mt-1">
                Create a workflow from a running sandbox or instance to automate recurring tasks
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

  if (!data) {
    return (
      <Card>
        <CardContent className="p-5">
          <div className="flex items-center gap-3">
            <div className="i-ph:flow-arrow text-lg text-cloud-elements-textTertiary" />
            <span className="text-sm font-data text-cloud-elements-textSecondary">Workflow #{String(id)}</span>
            <Badge variant="secondary">Loading...</Badge>
          </div>
        </CardContent>
      </Card>
    );
  }

  const triggerLabel: Record<string, string> = {
    cron: 'Cron',
    webhook: 'Webhook',
    manual: 'Manual',
  };

  const canRunActions = data.active && data.target_service_id !== '0';

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
                <h3 className="text-sm font-display font-semibold text-cloud-elements-textPrimary">{data.name || `Workflow #${String(id)}`}</h3>
                <Badge variant={data.active ? 'running' : 'secondary'}>
                  {data.active ? 'Active' : 'Inactive'}
                </Badge>
                <Badge variant="accent">
                  {triggerLabel[data.trigger_type] ?? data.trigger_type}
                </Badge>
                <Badge variant="secondary">{kindLabel}</Badge>
              </div>
              <div className="flex items-center gap-3 mt-1 text-xs text-cloud-elements-textTertiary flex-wrap">
                <span>Runs on {targetLabel}</span>
                {data.trigger_config && (
                  <>
                    <span className="text-cloud-elements-dividerColor">·</span>
                    <span className="font-data">{data.trigger_config}</span>
                  </>
                )}
                {data.last_triggered_at > 0 && (
                  <>
                    <span className="text-cloud-elements-dividerColor">·</span>
                    <span>Last: {new Date(data.last_triggered_at * 1000).toLocaleString()}</span>
                  </>
                )}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {canRunActions && (
              <>
                <Button variant="success" size="sm" onClick={onTrigger} disabled={txPending}>
                  <div className="i-ph:play text-xs" />
                  Trigger
                </Button>
                <Button variant="secondary" size="sm" onClick={onCancel} disabled={txPending}>
                  <div className="i-ph:stop text-xs" />
                  Cancel
                </Button>
              </>
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

function resolveWorkflowTargetLabel(
  workflow: WorkflowView | null,
  blueprintId: WorkflowBlueprintId,
  sandboxes: LocalSandbox[],
  instances: LocalInstance[],
) {
  if (!workflow) {
    return { label: 'Resolving target...', kindLabel: 'Workflow' };
  }

  if (workflow.target_kind === WORKFLOW_TARGET_SANDBOX) {
    const sandbox = sandboxes.find((record) => record.sandboxId === workflow.target_sandbox_id);
    return {
      label: sandbox?.name ?? workflow.target_sandbox_id ?? 'Unknown sandbox',
      kindLabel: 'Sandbox',
    };
  }

  const instance = instances.find((record) => {
    if (record.serviceId !== workflow.target_service_id) return false;
    if (blueprintId === 'ai-agent-tee-instance-blueprint') return !!record.teeEnabled;
    if (blueprintId === 'ai-agent-instance-blueprint') return !record.teeEnabled;
    return true;
  });

  return {
    label: instance?.name ?? `Service #${workflow.target_service_id}`,
    kindLabel: blueprintId === 'ai-agent-tee-instance-blueprint' ? 'TEE Instance' : 'Instance',
  };
}
