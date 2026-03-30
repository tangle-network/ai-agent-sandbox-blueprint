import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { toast } from 'sonner';
import { useStore } from '@nanostores/react';
import { useQueryClient } from '@tanstack/react-query';
import { useAccount } from 'wagmi';
import { AnimatedPage } from '@tangle-network/blueprint-ui/components';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { Input } from '@tangle-network/blueprint-ui/components';
import { Select } from '@tangle-network/blueprint-ui/components';
import { getAddresses, publicClient, tangleJobsAbi, useSubmitJob } from '@tangle-network/blueprint-ui';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { getJobById } from '@tangle-network/blueprint-ui';
import { normalizeAgentIdentifier } from '~/lib/agents';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { decodeEventLog, type Address } from 'viem';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import { INSTANCE_OPERATOR_API_URL, OPERATOR_API_URL } from '~/lib/config';
import { sandboxListStore } from '~/lib/stores/sandboxes';
import { instanceListStore } from '~/lib/stores/instances';
import {
  addPendingWorkflow,
  buildPendingWorkflowKey,
  normalizeWorkflowOwnerAddress,
  type PendingWorkflowCreation,
} from '~/lib/stores/pendingWorkflows';
import {
  WORKFLOW_TARGET_INSTANCE,
  WORKFLOW_TARGET_SANDBOX,
  buildWorkflowDetailPath,
  getWorkflowBlueprintIdForScope,
  getWorkflowScopeFromBlueprintId,
  type WorkflowBlueprintId,
  type WorkflowScope,
} from '~/lib/workflows';

const DEFAULT_WORKFLOW_JSON = '{\n  "prompt": ""\n}';
const DEFAULT_WORKFLOW_CONFIG_JSON = '{}';

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
  agentIdentifier: string;
};

type CreateState = 'idle' | 'signing' | 'confirming';

type CreatedWorkflow = {
  scope: WorkflowScope;
  workflowId: number;
  name: string;
};

function parseWorkflowCallId(
  logs: Array<{ data: `0x${string}`; topics: readonly `0x${string}`[] }>,
): number | null {
  for (const log of logs) {
    try {
      const decoded = decodeEventLog({
        abi: tangleJobsAbi,
        data: log.data,
        topics: [...log.topics] as [] | [`0x${string}`, ...`0x${string}`[]],
      });

      if (decoded.eventName === 'JobSubmitted' && 'callId' in decoded.args) {
        return Number(decoded.args.callId);
      }
    } catch {
      // Ignore unrelated logs while scanning the receipt.
    }
  }

  return null;
}

export default function WorkflowCreate() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [searchParams] = useSearchParams();
  const { address } = useAccount();
  const sandboxes = useStore(sandboxListStore);
  const instances = useStore(instanceListStore);
  const { submitJob, status: txStatus } = useSubmitJob();
  const addrs = getAddresses<SandboxAddresses>();
  const sandboxOperatorUrl = OPERATOR_API_URL;
  const instanceOperatorUrl = INSTANCE_OPERATOR_API_URL || OPERATOR_API_URL;

  const [name, setName] = useState('');
  const [selectedTargetKey, setSelectedTargetKey] = useState('');
  const [triggerType, setTriggerType] = useState('cron');
  const [triggerConfig, setTriggerConfig] = useState('');
  const [workflowJson, setWorkflowJson] = useState(DEFAULT_WORKFLOW_JSON);
  const [sandboxConfigJson, setSandboxConfigJson] = useState(DEFAULT_WORKFLOW_CONFIG_JSON);
  const [createError, setCreateError] = useState<string | null>(null);
  const [createState, setCreateState] = useState<CreateState>('idle');
  const [created, setCreated] = useState<CreatedWorkflow | null>(null);

  const normalizedOwnerAddress = useMemo(
    () => normalizeWorkflowOwnerAddress(address),
    [address],
  );

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
        agentIdentifier: normalizeAgentIdentifier(sandbox.agentIdentifier),
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
          agentIdentifier: normalizeAgentIdentifier(instance.agentIdentifier),
        };
      });

    return [...sandboxTargets, ...instanceTargets];
  }, [instances, sandboxes]);

  useEffect(() => {
    const requestedTarget = searchParams.get('target');
    if (!requestedTarget && availableTargets.length === 0) return;

    const normalizedRequested = requestedTarget ? decodeURIComponent(requestedTarget) : '';
    const targetExists = normalizedRequested
      ? availableTargets.some((target) => target.value === normalizedRequested)
      : false;

    if (targetExists) {
      setSelectedTargetKey(normalizedRequested);
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

  const selectedTargetHasAgent = selectedTarget != null && selectedTarget.agentIdentifier.length > 0;

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

  const handleCreate = useCallback(async () => {
    if (!address || !name || !selectedTarget) return;

    const job = getJobById(selectedTarget.blueprintId, JOB_IDS.WORKFLOW_CREATE);
    if (!job) return;

    setCreateError(null);

    if (triggerType === 'cron' && triggerConfig.trim()) {
      const fields = triggerConfig.trim().split(/\s+/);
      if (fields.length < 6 || fields.length > 7) {
        setCreateError(
          `Cron expression must have 6 or 7 fields (sec min hour dom mon dow [year]), got ${fields.length}. Example: 0 */5 * * * *`,
        );
        return;
      }
    }

    setCreateState('signing');

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
        toast.error('Transaction was not submitted');
        return;
      }

      setCreateState('confirming');

      const receipt = await publicClient.waitForTransactionReceipt({ hash });
      if (receipt.status === 'reverted') {
        throw new Error('Workflow creation transaction reverted.');
      }

      const workflowCallId = parseWorkflowCallId(
        receipt.logs as Array<{ data: `0x${string}`; topics: readonly `0x${string}`[] }>,
      );

      if (workflowCallId == null) {
        throw new Error('Transaction confirmed, but the workflow call ID could not be found.');
      }

      const scope = getWorkflowScopeFromBlueprintId(selectedTarget.blueprintId);
      const pending: PendingWorkflowCreation = {
        key: buildPendingWorkflowKey(address, scope, workflowCallId),
        ownerAddress: normalizedOwnerAddress,
        workflowId: workflowCallId,
        scope,
        blueprintId: selectedTarget.blueprintId,
        operatorUrl: scope === 'sandbox' ? sandboxOperatorUrl : instanceOperatorUrl,
        name,
        triggerType,
        triggerConfig,
        targetKind: selectedTarget.targetKind,
        targetSandboxId: selectedTarget.targetSandboxId,
        targetServiceId: Number(selectedTarget.serviceId),
        targetLabel: selectedTarget.label,
        kindLabel: selectedTarget.kindLabel,
        txHash: hash,
        createdAt: Date.now(),
        submittedAt: Date.now(),
        status: 'processing',
        statusMessage: 'Transaction confirmed. Waiting for the operator to publish the workflow.',
      };

      addPendingWorkflow(pending);
      await invalidateWorkflowQueries();
      setCreated({ scope, workflowId: workflowCallId, name });
      toast.success('Workflow created');
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Workflow creation failed';
      setCreateError(message);
      toast.error(message);
    } finally {
      setCreateState('idle');
    }
  }, [
    address,
    instanceOperatorUrl,
    invalidateWorkflowQueries,
    jobValue,
    name,
    normalizedOwnerAddress,
    sandboxConfigJson,
    sandboxOperatorUrl,
    selectedTarget,
    submitJob,
    triggerConfig,
    triggerType,
    workflowJson,
  ]);

  const triggerOptions = [
    { label: 'Cron Schedule', value: 'cron' },
    { label: 'Manual', value: 'manual' },
  ];

  const isCreateBusy = txStatus === 'pending' || txStatus === 'signing' || createState !== 'idle';
  const createButtonLabel = createState === 'signing'
    ? 'Awaiting Signature...'
    : createState === 'confirming'
      ? 'Confirming Transaction...'
      : 'Create Workflow';

  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">Create Workflow</h1>
          <p className="text-sm text-cloud-elements-textSecondary mt-1">
            Choose the resource this workflow will automate, then define the trigger and task payload.
          </p>
        </div>
        <Link to="/workflows">
          <Button variant="secondary">
            <div className="i-ph:arrow-left text-base" />
            Go Back
          </Button>
        </Link>
      </div>

      {!address && (
        <Card className="mb-6">
          <CardContent className="p-5">
            <div className="flex items-center gap-3">
              <div className="i-ph:wallet text-lg text-amber-400" />
              <div>
                <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                  Wallet not connected
                </p>
                <p className="text-xs text-cloud-elements-textTertiary mt-1">
                  Connect your wallet to create a workflow.
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {address && availableTargets.length === 0 && (
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

      {selectedTarget && !selectedTargetHasAgent && (
        <Card className="mb-6">
          <CardContent className="p-5">
            <div className="flex items-center gap-3">
              <div className="i-ph:warning text-lg text-amber-400" />
              <div>
                <p className="text-sm font-display font-medium text-cloud-elements-textPrimary">
                  No agent configured on this target
                </p>
                <p className="text-xs text-cloud-elements-textTertiary mt-1">
                  {selectedTarget.kindLabel} <strong>{selectedTarget.label}</strong> does not have an agent configured.
                  Workflow executions will fail until an agent is registered on the sidecar image.
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      <Card>
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
                  label: target.agentIdentifier
                    ? `${target.kindLabel}: ${target.label}`
                    : `${target.kindLabel}: ${target.label} (no agent)`,
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
                onValueChange={(value) => {
                  setTriggerType(value);
                  if (value !== 'cron') setTriggerConfig('');
                }}
                options={triggerOptions}
              />
            </div>
            {triggerType === 'cron' && (
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Cron Expression</label>
                <Input
                  value={triggerConfig}
                  onChange={(event) => setTriggerConfig(event.target.value)}
                  placeholder="0 */6 * * *"
                />
              </div>
            )}
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
            {created ? (
              <Button
                variant="success"
                onClick={() => navigate(buildWorkflowDetailPath(created.scope, created.workflowId))}
              >
                <div className="i-ph:check-bold text-sm" />
                View Workflow
              </Button>
            ) : (
              <Button onClick={handleCreate} disabled={!address || !name || !selectedTarget || isCreateBusy}>
                <div className="i-ph:flow-arrow text-sm" />
                {createButtonLabel}
              </Button>
            )}
          </div>

          {createError ? <p className="text-sm text-rose-400">{createError}</p> : null}
        </CardContent>
      </Card>
    </AnimatedPage>
  );
}
