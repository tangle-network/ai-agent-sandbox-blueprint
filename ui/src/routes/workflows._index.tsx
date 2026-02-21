import { useState, useCallback } from 'react';
import { AnimatedPage, StaggerContainer, StaggerItem } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Badge } from '~/components/ui/badge';
import { Input } from '~/components/ui/input';
import { Select } from '~/components/ui/select';
import { useWorkflowIds, useWorkflowBatch } from '~/lib/hooks/useSandboxReads';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { getJobById } from '~/lib/blueprints';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import { cn } from '~/lib/utils';

export default function Workflows() {
  const { data: workflowIds, isLoading } = useWorkflowIds(false);
  const { data: workflowData } = useWorkflowBatch(
    (workflowIds ?? []) as bigint[],
  );
  const { submitJob, status: txStatus } = useSubmitJob();

  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState('');
  const [triggerType, setTriggerType] = useState('cron');
  const [triggerConfig, setTriggerConfig] = useState('');
  const [workflowJson, setWorkflowJson] = useState('{}');
  const [sandboxConfigJson, setSandboxConfigJson] = useState('{}');
  const [serviceId, setServiceId] = useState('1');

  /** Compute job value from pricing tier (base rate = 0.001 TNT = 1e15 wei) */
  const jobValue = (jobId: number): bigint =>
    BigInt(PRICING_TIERS[jobId]?.multiplier ?? 1) * 1_000_000_000_000_000n;

  const handleCreate = useCallback(async () => {
    if (!name) return;
    const job = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.WORKFLOW_CREATE);
    if (!job) return;
    await submitJob({
      serviceId: BigInt(serviceId),
      jobId: JOB_IDS.WORKFLOW_CREATE,
      args: encodeJobArgs(job, { name, workflowJson, triggerType, triggerConfig, sandboxConfigJson }),
      label: `Create Workflow: ${name}`,
      value: jobValue(JOB_IDS.WORKFLOW_CREATE),
    });
    setShowCreate(false);
    setName('');
    setTriggerConfig('');
    setWorkflowJson('{}');
  }, [name, workflowJson, triggerType, triggerConfig, sandboxConfigJson, serviceId, submitJob]);

  const handleTrigger = useCallback(async (wfId: bigint) => {
    const job = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.WORKFLOW_TRIGGER);
    if (!job) return;
    await submitJob({
      serviceId: BigInt(serviceId),
      jobId: JOB_IDS.WORKFLOW_TRIGGER,
      args: encodeJobArgs(job, { workflowId: wfId }),
      label: `Trigger Workflow #${wfId}`,
      value: jobValue(JOB_IDS.WORKFLOW_TRIGGER),
    });
  }, [serviceId, submitJob]);

  const handleCancel = useCallback(async (wfId: bigint) => {
    const job = getJobById('ai-agent-sandbox-blueprint', JOB_IDS.WORKFLOW_CANCEL);
    if (!job) return;
    await submitJob({
      serviceId: BigInt(serviceId),
      jobId: JOB_IDS.WORKFLOW_CANCEL,
      args: encodeJobArgs(job, { workflowId: wfId }),
      label: `Cancel Workflow #${wfId}`,
      value: jobValue(JOB_IDS.WORKFLOW_CANCEL),
    });
  }, [serviceId, submitJob]);

  const workflows = (workflowIds ?? []).map((id, i) => {
    const data = workflowData?.[i];
    const result = data?.status === 'success' ? data.result : null;
    return { id: id as bigint, data: result };
  });

  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">Workflows</h1>
          <p className="text-sm text-cloud-elements-textSecondary mt-1">
            {workflows.length > 0 ? `${workflows.length} workflow${workflows.length > 1 ? 's' : ''}` : 'Scheduled tasks and automation'}
          </p>
        </div>
        <Button onClick={() => setShowCreate(!showCreate)}>
          <div className={showCreate ? 'i-ph:x text-base' : 'i-ph:plus text-base'} />
          {showCreate ? 'Cancel' : 'New Workflow'}
        </Button>
      </div>

      {/* Create Form */}
      {showCreate && (
        <Card className="mb-6">
          <CardHeader>
            <CardTitle>Create Workflow</CardTitle>
            <CardDescription>Define a scheduled or event-driven workflow</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Name</label>
                <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="daily-backup" />
              </div>
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Service ID</label>
                <Input type="number" value={serviceId} onChange={(e) => setServiceId(e.target.value)} min={1} />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Trigger Type</label>
                <Select
                  value={triggerType}
                  onValueChange={setTriggerType}
                  options={[
                    { label: 'Cron Schedule', value: 'cron' },
                    { label: 'Webhook', value: 'webhook' },
                    { label: 'Manual', value: 'manual' },
                  ]}
                />
              </div>
              <div>
                <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Trigger Config</label>
                <Input
                  value={triggerConfig}
                  onChange={(e) => setTriggerConfig(e.target.value)}
                  placeholder={triggerType === 'cron' ? '0 */6 * * *' : 'https://...'}
                />
              </div>
            </div>
            <div>
              <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Workflow Definition (JSON)</label>
              <textarea
                value={workflowJson}
                onChange={(e) => setWorkflowJson(e.target.value)}
                placeholder='{"steps":[...]}'
                rows={4}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
            </div>
            <div>
              <label className="block text-sm font-display font-medium text-cloud-elements-textSecondary mb-2">Sandbox Config (JSON)</label>
              <textarea
                value={sandboxConfigJson}
                onChange={(e) => setSandboxConfigJson(e.target.value)}
                placeholder='{"image":"ubuntu:22.04"}'
                rows={3}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm font-data text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
            </div>
            <div className="flex justify-end">
              <Button onClick={handleCreate} disabled={!name || txStatus === 'pending'}>
                <div className="i-ph:flow-arrow text-sm" />
                Create Workflow
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Workflow List */}
      {workflows.length > 0 ? (
        <StaggerContainer className="space-y-3">
          {workflows.map((wf) => (
            <StaggerItem key={String(wf.id)}>
              <WorkflowCard
                id={wf.id}
                data={wf.data}
                onTrigger={() => handleTrigger(wf.id)}
                onCancel={() => handleCancel(wf.id)}
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
                Create a workflow to schedule recurring agent tasks
              </p>
            </div>
          </CardContent>
        </Card>
      )}
    </AnimatedPage>
  );
}

interface WorkflowData {
  name: string;
  workflow_json: string;
  trigger_type: string;
  trigger_config: string;
  sandbox_config_json: string;
  active: boolean;
  created_at: bigint;
  updated_at: bigint;
  last_triggered_at: bigint;
}

function WorkflowCard({
  id,
  data,
  onTrigger,
  onCancel,
  txPending,
}: {
  id: bigint;
  data: WorkflowData | null | undefined;
  onTrigger: () => void;
  onCancel: () => void;
  txPending: boolean;
}) {
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

  return (
    <Card>
      <CardContent className="p-5">
        <div className="flex items-center justify-between">
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
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-display font-semibold text-cloud-elements-textPrimary">{data.name || `Workflow #${String(id)}`}</h3>
                <Badge variant={data.active ? 'running' : 'secondary'}>
                  {data.active ? 'Active' : 'Inactive'}
                </Badge>
                <Badge variant="accent">
                  {triggerLabel[data.trigger_type] ?? data.trigger_type}
                </Badge>
              </div>
              <div className="flex items-center gap-3 mt-1 text-xs text-cloud-elements-textTertiary">
                {data.trigger_config && (
                  <span className="font-data">{data.trigger_config}</span>
                )}
                {data.last_triggered_at > 0n && (
                  <>
                    <span className="text-cloud-elements-dividerColor">·</span>
                    <span>Last: {new Date(Number(data.last_triggered_at) * 1000).toLocaleString()}</span>
                  </>
                )}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {data.active && (
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
