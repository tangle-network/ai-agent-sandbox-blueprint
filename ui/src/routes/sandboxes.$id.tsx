import { useParams, Link } from 'react-router';
import { lazy, Suspense, useState, useCallback, useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { JobPriceBadge } from '~/components/shared/JobPriceBadge';
import { sandboxListStore, updateSandboxStatus, getSandbox } from '~/lib/stores/sandboxes';
import { useSandboxActive, useSandboxOperator } from '~/lib/hooks/useSandboxReads';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { encodeJobArgs } from '~/lib/contracts/generic-encoder';
import { getJobById } from '~/lib/blueprints';
import { JOB_IDS, PRICING_TIERS } from '~/lib/types/sandbox';
import '~/lib/blueprints'; // auto-register
import { ChatContainer, type AgentBranding } from '@tangle/agent-ui';
import { useSandboxChat } from '~/lib/hooks/useSandboxChat';
import { useWagmiSidecarAuth } from '~/lib/hooks/useWagmiSidecarAuth';
import { createDirectClient, type SandboxClient } from '~/lib/api/sandboxClient';
import { cn } from '~/lib/utils';

const TerminalView = lazy(() =>
  import('@tangle/agent-ui/terminal').then((m) => ({ default: m.TerminalView }))
);

type ActionTab = 'overview' | 'terminal' | 'prompt' | 'task' | 'ssh';

// Branding presets for each tab's chat container
const PROMPT_BRANDING: AgentBranding = {
  label: 'Agent',
  accentClass: 'text-violet-600 dark:text-violet-400',
  bgClass: 'bg-violet-500/5',
  containerBgClass: 'bg-violet-50/40 dark:bg-neutral-950/60',
  borderClass: 'border-violet-500/15 dark:border-violet-500/20',
  iconClass: 'i-ph:robot',
  textClass: 'text-violet-600 dark:text-violet-400',
};

const TASK_BRANDING: AgentBranding = {
  label: 'Task Agent',
  accentClass: 'text-amber-600 dark:text-amber-400',
  bgClass: 'bg-amber-500/5',
  containerBgClass: 'bg-amber-50/40 dark:bg-neutral-950/60',
  borderClass: 'border-amber-500/15 dark:border-amber-500/20',
  iconClass: 'i-ph:lightning',
  textClass: 'text-amber-600 dark:text-amber-400',
};

export default function SandboxDetail() {
  const { id } = useParams<{ id: string }>();
  const decodedId = id ? decodeURIComponent(id) : '';
  const sandboxes = useStore(sandboxListStore);
  const sb = sandboxes.find((s) => s.id === decodedId);

  const { data: isActive } = useSandboxActive(decodedId);
  const { data: operator } = useSandboxOperator(decodedId);
  const { submitJob, status: txStatus, txHash } = useSubmitJob();

  const [tab, setTab] = useState<ActionTab>('overview');
  const [systemPrompt, setSystemPrompt] = useState('');

  const serviceId = BigInt(sb?.serviceId ?? '1');

  // Sidecar auth for PTY terminal and API access
  const sidecarUrl = sb?.sidecarUrl ?? '';
  const { token: sidecarToken, isAuthenticated: isSidecarAuthed, authenticate: sidecarAuth, isAuthenticating } = useWagmiSidecarAuth(decodedId, sidecarUrl);

  // Create sandbox client for direct API access (uses authenticated sidecar token)
  const client: SandboxClient | null = useMemo(() => {
    if (!sb?.sidecarUrl || !sidecarToken) return null;
    return createDirectClient(sb.sidecarUrl, sidecarToken);
  }, [sb?.sidecarUrl, sidecarToken]);

  // Chat hooks for prompt/task tabs
  const promptChat = useSandboxChat({ client, mode: 'prompt', systemPrompt });
  const taskChat = useSandboxChat({ client, mode: 'task', systemPrompt });

  const bpId = 'ai-agent-sandbox-blueprint';

  /** Compute job value from pricing tier (base rate = 0.001 TNT = 1e15 wei) */
  const jobValue = (jobId: number): bigint =>
    BigInt(PRICING_TIERS[jobId]?.multiplier ?? 1) * 1_000_000_000_000_000n;

  const encodeCtxJob = useCallback(
    (jobId: number, ctx: Record<string, unknown>, formValues: Record<string, unknown> = {}) => {
      const job = getJobById(bpId, jobId);
      if (!job) throw new Error(`Job ${jobId} not found`);
      return encodeJobArgs(job, formValues, ctx);
    },
    [],
  );

  const handleStop = useCallback(async () => {
    const hash = await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_STOP,
      args: encodeCtxJob(JOB_IDS.SANDBOX_STOP, { sandbox_id: decodedId }),
      label: `Stop: ${decodedId}`,
      value: jobValue(JOB_IDS.SANDBOX_STOP),
    });
    if (hash) updateSandboxStatus(decodedId, 'stopped');
  }, [decodedId, serviceId, submitJob, encodeCtxJob]);

  const handleResume = useCallback(async () => {
    const hash = await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_RESUME,
      args: encodeCtxJob(JOB_IDS.SANDBOX_RESUME, { sandbox_id: decodedId }),
      label: `Resume: ${decodedId}`,
      value: jobValue(JOB_IDS.SANDBOX_RESUME),
    });
    if (hash) updateSandboxStatus(decodedId, 'running');
  }, [decodedId, serviceId, submitJob, encodeCtxJob]);

  const handleDelete = useCallback(async () => {
    const hash = await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_DELETE,
      args: encodeCtxJob(JOB_IDS.SANDBOX_DELETE, { sandbox_id: decodedId }),
      label: `Delete: ${decodedId}`,
      value: jobValue(JOB_IDS.SANDBOX_DELETE),
    });
    if (hash) updateSandboxStatus(decodedId, 'gone');
  }, [decodedId, serviceId, submitJob, encodeCtxJob]);

  const handleSnapshot = useCallback(async () => {
    const sidecarCtx = { sidecar_url: sb?.sidecarUrl ?? '' };
    await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_SNAPSHOT,
      args: encodeCtxJob(JOB_IDS.SANDBOX_SNAPSHOT, sidecarCtx, {
        destination: '',
        includeWorkspace: true,
        includeState: true,
      }),
      label: `Snapshot: ${decodedId}`,
      value: jobValue(JOB_IDS.SANDBOX_SNAPSHOT),
    });
  }, [decodedId, serviceId, sb?.sidecarUrl, submitJob, encodeCtxJob]);

  if (!sb) {
    return (
      <AnimatedPage className="mx-auto max-w-3xl px-4 sm:px-6 py-8">
        <Card>
          <CardContent className="p-6 text-center py-16">
            <div className="i-ph:hard-drives text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
            <p className="text-cloud-elements-textSecondary font-display">Sandbox not found</p>
            <Link to="/sandboxes" className="inline-block mt-4">
              <Button variant="outline" size="sm">Back to Sandboxes</Button>
            </Link>
          </CardContent>
        </Card>
      </AnimatedPage>
    );
  }

  const isRunning = sb.status === 'running' || sb.status === 'creating';
  const isStopped = sb.status === 'stopped' || sb.status === 'warm';
  const isGone = sb.status === 'gone';

  const tabs: { key: ActionTab; label: string; icon: string; disabled?: boolean }[] = [
    { key: 'overview', label: 'Overview', icon: 'i-ph:info' },
    { key: 'terminal', label: 'Terminal', icon: 'i-ph:terminal', disabled: !isRunning },
    { key: 'prompt', label: 'Prompt', icon: 'i-ph:robot', disabled: !isRunning },
    { key: 'task', label: 'Task', icon: 'i-ph:lightning', disabled: !isRunning },
    { key: 'ssh', label: 'SSH', icon: 'i-ph:key', disabled: !isRunning },
  ];

  return (
    <AnimatedPage className="mx-auto max-w-4xl px-4 sm:px-6 py-8">
      {/* Header */}
      <div className="flex items-center gap-2 mb-6 text-sm text-cloud-elements-textTertiary">
        <Link to="/sandboxes" className="hover:text-cloud-elements-textSecondary transition-colors">Sandboxes</Link>
        <span>/</span>
        <span className="text-cloud-elements-textPrimary font-display">{sb.name}</span>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div className="flex items-center gap-4">
          <div className={cn(
            'w-14 h-14 rounded-xl flex items-center justify-center',
            isRunning ? 'bg-teal-500/10' : isStopped ? 'bg-amber-500/10' : 'bg-cloud-elements-background-depth-3',
          )}>
            <div className={cn(
              'i-ph:hard-drives text-2xl',
              isRunning ? 'text-teal-400' : isStopped ? 'text-amber-400' : 'text-cloud-elements-textTertiary',
            )} />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-xl font-display font-bold text-cloud-elements-textPrimary">{sb.name}</h1>
              <StatusBadge status={sb.status === 'creating' ? 'running' : sb.status} />
            </div>
            <div className="flex items-center gap-3 mt-1">
              <span className="text-xs font-data text-cloud-elements-textTertiary">{sb.image}</span>
              <span className="text-cloud-elements-dividerColor">·</span>
              <span className="text-xs font-data text-cloud-elements-textTertiary">
                {sb.cpuCores} CPU · {sb.memoryMb}MB · {sb.diskGb}GB
              </span>
            </div>
          </div>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          {isRunning && (
            <Button variant="secondary" size="sm" onClick={handleStop}>
              <div className="i-ph:stop text-sm" />
              Stop
              <JobPriceBadge jobIndex={JOB_IDS.SANDBOX_STOP} pricingMultiplier={PRICING_TIERS[JOB_IDS.SANDBOX_STOP]?.multiplier ?? 1} compact />
            </Button>
          )}
          {isStopped && (
            <Button variant="success" size="sm" onClick={handleResume}>
              <div className="i-ph:play text-sm" />
              Resume
              <JobPriceBadge jobIndex={JOB_IDS.SANDBOX_RESUME} pricingMultiplier={PRICING_TIERS[JOB_IDS.SANDBOX_RESUME]?.multiplier ?? 1} compact />
            </Button>
          )}
          {!isGone && (
            <>
              <Button variant="secondary" size="sm" onClick={handleSnapshot}>
                <div className="i-ph:camera text-sm" />
                Snapshot
                <JobPriceBadge jobIndex={JOB_IDS.SANDBOX_SNAPSHOT} pricingMultiplier={PRICING_TIERS[JOB_IDS.SANDBOX_SNAPSHOT]?.multiplier ?? 5} compact />
              </Button>
              <Button variant="destructive" size="sm" onClick={handleDelete}>
                <div className="i-ph:trash text-sm" />
                Delete
                <JobPriceBadge jobIndex={JOB_IDS.SANDBOX_DELETE} pricingMultiplier={PRICING_TIERS[JOB_IDS.SANDBOX_DELETE]?.multiplier ?? 1} compact />
              </Button>
            </>
          )}
        </div>
      </div>

      {/* Tabs */}
      <div className="flex items-center gap-1 mb-6 border-b border-cloud-elements-dividerColor pb-px">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => !t.disabled && setTab(t.key)}
            disabled={t.disabled}
            className={cn(
              'flex items-center gap-1.5 px-3 py-2 text-sm font-display font-medium transition-colors border-b-2 -mb-px',
              tab === t.key
                ? 'text-violet-700 dark:text-violet-400 border-violet-500'
                : t.disabled
                  ? 'text-cloud-elements-textTertiary border-transparent cursor-not-allowed opacity-50'
                  : 'text-cloud-elements-textSecondary border-transparent hover:text-cloud-elements-textPrimary hover:border-cloud-elements-borderColor',
            )}
          >
            <div className={`${t.icon} text-sm`} />
            {t.label}
          </button>
        ))}
      </div>

      {/* Tab Content */}
      {tab === 'overview' && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Configuration</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2.5">
              <DetailRow label="Sandbox ID" value={sb.id} mono />
              <DetailRow label="Image" value={sb.image} mono />
              <DetailRow label="CPU" value={`${sb.cpuCores} cores`} />
              <DetailRow label="Memory" value={`${sb.memoryMb} MB`} />
              <DetailRow label="Disk" value={`${sb.diskGb} GB`} />
              <DetailRow label="Created" value={new Date(sb.createdAt).toLocaleString()} />
              <DetailRow label="Blueprint" value={sb.blueprintId} mono />
              <DetailRow label="Service ID" value={sb.serviceId} />
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm">On-Chain Status</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2.5">
              <DetailRow label="Active" value={isActive !== undefined ? (isActive ? 'Yes' : 'No') : 'Loading...'} />
              <DetailRow
                label="Operator"
                value={operator && operator !== '0x0000000000000000000000000000000000000000' ? operator : 'Unassigned'}
                mono
              />
              {sb.txHash && <DetailRow label="TX Hash" value={sb.txHash} mono />}
              {sb.sidecarUrl && <DetailRow label="Sidecar" value={sb.sidecarUrl} mono />}
            </CardContent>
          </Card>
        </div>
      )}

      {/* Terminal Tab — real PTY via sidecar */}
      {tab === 'terminal' && (
        <Card className="overflow-hidden">
          {!isSidecarAuthed ? (
            <CardContent className="py-16 text-center">
              <div className="i-ph:terminal-window text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary mb-2">
                Authenticate to access the sandbox terminal
              </p>
              <p className="text-xs text-cloud-elements-textTertiary mb-4">
                You'll be asked to sign a message with your wallet to verify ownership
              </p>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => sidecarAuth()}
                disabled={isAuthenticating || !sidecarUrl}
              >
                {isAuthenticating ? 'Signing...' : !sidecarUrl ? 'Waiting for sidecar...' : 'Connect Terminal'}
              </Button>
            </CardContent>
          ) : (
            <CardContent className="p-0">
              <div className="h-[500px]">
                <Suspense fallback={
                  <div className="flex items-center justify-center h-full bg-neutral-950">
                    <span className="text-sm text-neutral-500">Loading terminal...</span>
                  </div>
                }>
                  <TerminalView
                    apiUrl={sidecarUrl}
                    token={sidecarToken!}
                    title="Sandbox Terminal"
                    subtitle="Connected to sidecar PTY session"
                  />
                </Suspense>
              </div>
            </CardContent>
          )}
        </Card>
      )}

      {/* Prompt Tab — ChatContainer with agent prompting */}
      {tab === 'prompt' && (
        <div className="space-y-4">
          {/* System prompt config */}
          <Card>
            <CardContent className="p-3">
              <details className="group">
                <summary className="cursor-pointer text-xs font-display text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary transition-colors flex items-center gap-1.5">
                  <div className="i-ph:gear text-sm" />
                  System Prompt
                  {systemPrompt && <span className="text-violet-400 ml-1">(set)</span>}
                </summary>
                <textarea
                  value={systemPrompt}
                  onChange={(e) => setSystemPrompt(e.target.value)}
                  placeholder="You are a helpful coding assistant."
                  rows={2}
                  className="mt-2 flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
                />
              </details>
            </CardContent>
          </Card>

          <Card className="overflow-hidden">
            <CardContent className="p-0">
              <div className="h-[500px]">
                <ChatContainer
                  messages={promptChat.messages}
                  partMap={promptChat.partMap}
                  isStreaming={promptChat.isStreaming}
                  onSend={promptChat.send}
                  branding={PROMPT_BRANDING}
                  placeholder="What files are in the workspace?"
                />
              </div>
            </CardContent>
          </Card>
        </div>
      )}

      {/* Task Tab — ChatContainer for autonomous tasks */}
      {tab === 'task' && (
        <div className="space-y-4">
          <Card>
            <CardContent className="p-3">
              <details className="group">
                <summary className="cursor-pointer text-xs font-display text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary transition-colors flex items-center gap-1.5">
                  <div className="i-ph:gear text-sm" />
                  System Prompt
                  {systemPrompt && <span className="text-amber-400 ml-1">(set)</span>}
                </summary>
                <textarea
                  value={systemPrompt}
                  onChange={(e) => setSystemPrompt(e.target.value)}
                  placeholder="You are an expert developer."
                  rows={2}
                  className="mt-2 flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
                />
              </details>
            </CardContent>
          </Card>

          <Card className="overflow-hidden">
            <CardContent className="p-0">
              <div className="h-[500px]">
                <ChatContainer
                  messages={taskChat.messages}
                  partMap={taskChat.partMap}
                  isStreaming={taskChat.isStreaming}
                  onSend={taskChat.send}
                  branding={TASK_BRANDING}
                  placeholder="Build a REST API with Express.js..."
                />
              </div>
            </CardContent>
          </Card>
        </div>
      )}

      {tab === 'ssh' && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">SSH Access</CardTitle>
            <CardDescription>Manage SSH keys for this sandbox</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="py-8 text-center">
              <div className="i-ph:key text-3xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-sm text-cloud-elements-textSecondary">SSH key management coming soon</p>
              <p className="text-xs text-cloud-elements-textTertiary mt-1">
                Provision and revoke SSH keys for secure remote access
              </p>
            </div>
          </CardContent>
        </Card>
      )}
    </AnimatedPage>
  );
}

function DetailRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex justify-between text-sm gap-2">
      <span className="text-cloud-elements-textSecondary shrink-0">{label}</span>
      <span className={cn('text-cloud-elements-textPrimary truncate text-right', mono && 'font-data text-xs')}>
        {value}
      </span>
    </div>
  );
}
