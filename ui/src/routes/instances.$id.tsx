import { useParams, Link } from 'react-router';
import { lazy, Suspense, useState, useCallback, useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { instanceListStore, updateInstanceStatus } from '~/lib/stores/instances';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { getBlueprint, getJobById } from '~/lib/blueprints';
import { INSTANCE_JOB_IDS, INSTANCE_PRICING_TIERS } from '~/lib/types/instance';
import { ChatContainer, type AgentBranding } from '@tangle/agent-ui';
import { useWagmiSidecarAuth } from '~/lib/hooks/useWagmiSidecarAuth';
import { createDirectClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { useSandboxChat } from '~/lib/hooks/useSandboxChat';
import { cn } from '~/lib/utils';

const TerminalView = lazy(() =>
  import('@tangle/agent-ui/terminal').then((m) => ({ default: m.TerminalView })),
);

type ActionTab = 'overview' | 'terminal' | 'prompt' | 'task';

const PROMPT_BRANDING: AgentBranding = {
  label: 'Agent',
  accentClass: 'text-blue-400',
  bgClass: 'bg-blue-500/5',
  containerBgClass: 'bg-neutral-950/60',
  borderClass: 'border-blue-500/20',
  iconClass: 'i-ph:robot',
  textClass: 'text-blue-400',
};

const TASK_BRANDING: AgentBranding = {
  label: 'Task Agent',
  accentClass: 'text-amber-400',
  bgClass: 'bg-amber-500/5',
  containerBgClass: 'bg-neutral-950/60',
  borderClass: 'border-amber-500/20',
  iconClass: 'i-ph:lightning',
  textClass: 'text-amber-400',
};

export default function InstanceDetail() {
  const { id } = useParams<{ id: string }>();
  const decodedId = id ? decodeURIComponent(id) : '';
  const instances = useStore(instanceListStore);
  const inst = instances.find((s) => s.id === decodedId);

  const { submitJob, status: txStatus } = useSubmitJob();
  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');

  const serviceId = BigInt(inst?.serviceId ?? '1');
  const bpId = inst?.teeEnabled ? 'ai-agent-tee-instance-blueprint' : 'ai-agent-instance-blueprint';

  // Sidecar auth
  const sidecarUrl = inst?.sidecarUrl ?? '';
  const { token: sidecarToken, isAuthenticated: isSidecarAuthed, authenticate: sidecarAuth, isAuthenticating } =
    useWagmiSidecarAuth(decodedId, sidecarUrl);

  const client: SandboxClient | null = useMemo(() => {
    if (!sidecarUrl || !sidecarToken) return null;
    return createDirectClient(sidecarUrl, sidecarToken);
  }, [sidecarUrl, sidecarToken]);

  const promptChat = useSandboxChat({ client, mode: 'prompt', systemPrompt });
  const taskChat = useSandboxChat({ client, mode: 'task', systemPrompt });

  /** Compute job value from pricing tier (base rate = 0.001 TNT = 1e15 wei) */
  const jobValue = (jobId: number): bigint =>
    BigInt(INSTANCE_PRICING_TIERS[jobId]?.multiplier ?? 1) * 1_000_000_000_000_000n;

  const handleDeprovision = useCallback(async () => {
    const job = getJobById(bpId, INSTANCE_JOB_IDS.DEPROVISION);
    if (!job) return;
    const args = encodeJobArgs(job, { json: '{}' });
    const hash = await submitJob({
      serviceId,
      jobId: INSTANCE_JOB_IDS.DEPROVISION,
      args,
      label: 'Deprovision Instance',
      value: jobValue(INSTANCE_JOB_IDS.DEPROVISION),
    });
    if (hash) updateInstanceStatus(decodedId, 'gone');
  }, [bpId, serviceId, decodedId, submitJob]);

  if (!inst) {
    return (
      <AnimatedPage className="mx-auto max-w-3xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center">
            <div className="i-ph:cube text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
            <p className="text-cloud-elements-textSecondary">Instance not found</p>
            <Link to="/instances" className="inline-block mt-4">
              <Button variant="outline" size="sm">Back to Instances</Button>
            </Link>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  const tabs: { key: ActionTab; label: string; icon: string }[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal' },
    { key: 'prompt', label: 'Prompt', icon: 'i-ph:robot' },
    { key: 'task', label: 'Task', icon: 'i-ph:lightning' },
  ];

  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-4">
          <Link to="/instances">
            <Button variant="ghost" size="sm">
              <div className="i-ph:arrow-left text-base" />
            </Button>
          </Link>
          <div>
            <div className="flex items-center gap-3">
              <h1 className="text-xl font-display font-bold text-cloud-elements-textPrimary">{inst.name}</h1>
              <StatusBadge status={inst.status === 'creating' ? 'running' : inst.status} />
              {inst.teeEnabled && (
                <span className="text-xs text-violet-400 font-data bg-violet-500/10 px-2 py-0.5 rounded-full">TEE</span>
              )}
            </div>
            <p className="text-xs font-data text-cloud-elements-textTertiary mt-1">
              {inst.image} · {inst.cpuCores} CPU · {inst.memoryMb}MB
            </p>
          </div>
        </div>
        <Button variant="destructive" size="sm" onClick={handleDeprovision} disabled={txStatus !== 'idle'}>
          <div className="i-ph:trash text-sm" />
          Deprovision
          <JobPriceBadge jobIndex={INSTANCE_JOB_IDS.DEPROVISION} pricingMultiplier={INSTANCE_PRICING_TIERS[INSTANCE_JOB_IDS.DEPROVISION]?.multiplier ?? 1} compact />
        </Button>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 mb-6 border-b border-cloud-elements-dividerColor">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={cn(
              'flex items-center gap-2 px-4 py-2.5 text-sm font-display font-medium transition-colors border-b-2 -mb-px',
              tab === t.key
                ? 'border-violet-500 text-cloud-elements-textPrimary'
                : 'border-transparent text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary',
            )}
          >
            <div className={`${t.icon} text-base`} />
            {t.label}
          </button>
        ))}
      </div>

      {/* Overview */}
      {tab === 'overview' && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <Card>
            <CardHeader>
              <CardTitle>Instance Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <Row label="ID" value={inst.id} mono />
              <Row label="Image" value={inst.image} mono />
              <Row label="CPU" value={`${inst.cpuCores} cores`} />
              <Row label="Memory" value={`${inst.memoryMb} MB`} />
              <Row label="Disk" value={`${inst.diskGb} GB`} />
              <Row label="Created" value={new Date(inst.createdAt).toLocaleString()} />
              <Row label="Blueprint" value={getBlueprint(bpId)?.name ?? bpId} />
              <Row label="Service" value={`#${inst.serviceId}`} />
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Connection</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <Row label="Sidecar URL" value={inst.sidecarUrl ?? 'Pending...'} mono />
              <Row label="Authenticated" value={isSidecarAuthed ? 'Yes' : 'No'} />
              {!isSidecarAuthed && inst.sidecarUrl && (
                <Button size="sm" onClick={sidecarAuth} disabled={isAuthenticating}>
                  {isAuthenticating ? 'Authenticating...' : 'Authenticate'}
                </Button>
              )}
            </CardContent>
          </Card>
        </div>
      )}

      {/* Terminal */}
      {tab === 'terminal' && (
        <Card className="overflow-hidden">
          <CardContent className="p-0">
            {isSidecarAuthed && sidecarUrl ? (
              <Suspense fallback={<div className="p-6 text-sm text-cloud-elements-textTertiary">Loading terminal...</div>}>
                <div className="h-[500px]">
                  <TerminalView
                    apiUrl={sidecarUrl}
                    token={sidecarToken!}
                    title="Instance Terminal"
                    subtitle="Connected to sidecar PTY session"
                  />
                </div>
              </Suspense>
            ) : (
              <div className="p-6 text-center">
                <p className="text-sm text-cloud-elements-textSecondary mb-3">
                  Authenticate to access the terminal
                </p>
                <Button size="sm" onClick={sidecarAuth} disabled={isAuthenticating || !sidecarUrl}>
                  {isAuthenticating ? 'Authenticating...' : 'Authenticate'}
                </Button>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* Prompt */}
      {tab === 'prompt' && (
        <Card className="overflow-hidden">
          <CardContent className="p-0 h-[600px]">
            <ChatContainer
              messages={promptChat.messages}
              partMap={promptChat.partMap}
              isStreaming={promptChat.isStreaming}
              onSend={promptChat.send}
              branding={PROMPT_BRANDING}
              placeholder="Send a prompt to the agent..."
            />
          </CardContent>
        </Card>
      )}

      {/* Task */}
      {tab === 'task' && (
        <Card className="overflow-hidden">
          <CardContent className="p-0 h-[600px]">
            <ChatContainer
              messages={taskChat.messages}
              partMap={taskChat.partMap}
              isStreaming={taskChat.isStreaming}
              onSend={taskChat.send}
              branding={TASK_BRANDING}
              placeholder="Describe a task for the agent..."
            />
          </CardContent>
        </Card>
      )}
    </AnimatedPage>
  );
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex justify-between text-sm">
      <span className="text-cloud-elements-textSecondary">{label}</span>
      <span className={cn('text-cloud-elements-textPrimary', mono && 'font-data text-xs')}>{value}</span>
    </div>
  );
}
