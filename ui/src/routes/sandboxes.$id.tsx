import { useParams, useNavigate, Link } from 'react-router';
import { useState, useCallback } from 'react';
import { useStore } from '@nanostores/react';
import { AnimatedPage } from '~/components/motion/AnimatedPage';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Badge } from '~/components/ui/badge';
import { Input } from '~/components/ui/input';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { sandboxListStore, updateSandboxStatus, removeSandbox, getSandbox } from '~/lib/stores/sandboxes';
import { useSandboxActive, useSandboxOperator } from '~/lib/hooks/useSandboxReads';
import { useSubmitJob } from '~/lib/hooks/useSubmitJob';
import { encodeSandboxId, encodeSnapshot, encodeExec, encodePrompt, encodeTask } from '~/lib/contracts/encoding';
import { JOB_IDS } from '~/lib/types/sandbox';
import { Identicon } from '~/components/shared/Identicon';
import { cn } from '~/lib/utils';

type ActionTab = 'overview' | 'terminal' | 'prompt' | 'task' | 'ssh';

export default function SandboxDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const decodedId = id ? decodeURIComponent(id) : '';
  const sandbox = getSandbox(decodedId);
  const sandboxes = useStore(sandboxListStore); // reactive
  const sb = sandboxes.find((s) => s.id === decodedId);

  const { data: isActive } = useSandboxActive(decodedId);
  const { data: operator } = useSandboxOperator(decodedId);
  const { submitJob, status: txStatus, txHash, reset: resetTx } = useSubmitJob();

  const [tab, setTab] = useState<ActionTab>('overview');
  const [command, setCommand] = useState('');
  const [prompt, setPrompt] = useState('');
  const [taskText, setTaskText] = useState('');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [snapshotTier, setSnapshotTier] = useState('hot');

  const serviceId = BigInt(sb?.serviceId ?? '1');

  const handleStop = useCallback(async () => {
    const hash = await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_STOP,
      args: encodeSandboxId(decodedId),
      label: `Stop: ${decodedId}`,
    });
    if (hash) updateSandboxStatus(decodedId, 'stopped');
  }, [decodedId, serviceId, submitJob]);

  const handleResume = useCallback(async () => {
    const hash = await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_RESUME,
      args: encodeSandboxId(decodedId),
      label: `Resume: ${decodedId}`,
    });
    if (hash) updateSandboxStatus(decodedId, 'running');
  }, [decodedId, serviceId, submitJob]);

  const handleDelete = useCallback(async () => {
    const hash = await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_DELETE,
      args: encodeSandboxId(decodedId),
      label: `Delete: ${decodedId}`,
    });
    if (hash) updateSandboxStatus(decodedId, 'gone');
  }, [decodedId, serviceId, submitJob]);

  const handleSnapshot = useCallback(async () => {
    await submitJob({
      serviceId,
      jobId: JOB_IDS.SANDBOX_SNAPSHOT,
      args: encodeSnapshot(decodedId, snapshotTier),
      label: `Snapshot: ${decodedId} (${snapshotTier})`,
    });
  }, [decodedId, serviceId, snapshotTier, submitJob]);

  const handleExec = useCallback(async () => {
    if (!command) return;
    await submitJob({
      serviceId,
      jobId: JOB_IDS.EXEC,
      args: encodeExec(decodedId, command),
      label: `Exec: ${command}`,
    });
    setCommand('');
  }, [decodedId, serviceId, command, submitJob]);

  const handlePrompt = useCallback(async () => {
    if (!prompt) return;
    await submitJob({
      serviceId,
      jobId: JOB_IDS.PROMPT,
      args: encodePrompt(decodedId, prompt, systemPrompt || undefined),
      label: `Prompt: ${prompt.slice(0, 40)}...`,
    });
    setPrompt('');
  }, [decodedId, serviceId, prompt, systemPrompt, submitJob]);

  const handleTask = useCallback(async () => {
    if (!taskText) return;
    await submitJob({
      serviceId,
      jobId: JOB_IDS.TASK,
      args: encodeTask(decodedId, taskText, systemPrompt || undefined),
      label: `Task: ${taskText.slice(0, 40)}...`,
    });
    setTaskText('');
  }, [decodedId, serviceId, taskText, systemPrompt, submitJob]);

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
            </Button>
          )}
          {isStopped && (
            <Button variant="success" size="sm" onClick={handleResume}>
              <div className="i-ph:play text-sm" />
              Resume
            </Button>
          )}
          {!isGone && (
            <>
              <Button variant="secondary" size="sm" onClick={handleSnapshot}>
                <div className="i-ph:camera text-sm" />
                Snapshot
              </Button>
              <Button variant="destructive" size="sm" onClick={handleDelete}>
                <div className="i-ph:trash text-sm" />
                Delete
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

      {tab === 'terminal' && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Execute Command</CardTitle>
            <CardDescription>Run a shell command inside the sandbox</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex gap-2">
              <Input
                value={command}
                onChange={(e) => setCommand(e.target.value)}
                placeholder="ls -la /workspace"
                onKeyDown={(e) => e.key === 'Enter' && handleExec()}
                className="font-data"
              />
              <Button onClick={handleExec} disabled={!command || txStatus === 'pending'}>
                <div className="i-ph:play text-sm" />
                Run
              </Button>
            </div>
            <TxStatusIndicator status={txStatus} hash={txHash} />
          </CardContent>
        </Card>
      )}

      {tab === 'prompt' && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">AI Prompt</CardTitle>
            <CardDescription>Send a prompt to the sandbox agent (20x base rate)</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <label className="block text-xs font-display text-cloud-elements-textTertiary mb-1">System Prompt (optional)</label>
              <textarea
                value={systemPrompt}
                onChange={(e) => setSystemPrompt(e.target.value)}
                placeholder="You are a helpful coding assistant."
                rows={2}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
            </div>
            <div>
              <label className="block text-xs font-display text-cloud-elements-textTertiary mb-1">Prompt</label>
              <textarea
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                placeholder="What files are in the workspace?"
                rows={3}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
            </div>
            <div className="flex justify-end">
              <Button onClick={handlePrompt} disabled={!prompt || txStatus === 'pending'}>
                <div className="i-ph:robot text-sm" />
                Send Prompt
              </Button>
            </div>
            <TxStatusIndicator status={txStatus} hash={txHash} />
          </CardContent>
        </Card>
      )}

      {tab === 'task' && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Autonomous Task</CardTitle>
            <CardDescription>Submit an agent task for autonomous completion (250x base rate)</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <label className="block text-xs font-display text-cloud-elements-textTertiary mb-1">System Prompt (optional)</label>
              <textarea
                value={systemPrompt}
                onChange={(e) => setSystemPrompt(e.target.value)}
                placeholder="You are an expert developer."
                rows={2}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
            </div>
            <div>
              <label className="block text-xs font-display text-cloud-elements-textTertiary mb-1">Task Description</label>
              <textarea
                value={taskText}
                onChange={(e) => setTaskText(e.target.value)}
                placeholder="Build a REST API with Express.js that has CRUD endpoints for users..."
                rows={5}
                className="flex w-full rounded-lg border border-cloud-elements-borderColor bg-cloud-elements-background-depth-2 px-3 py-2 text-sm text-cloud-elements-textPrimary placeholder:text-cloud-elements-textTertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400/50 resize-y"
              />
            </div>
            <div className="flex justify-end">
              <Button onClick={handleTask} disabled={!taskText || txStatus === 'pending'}>
                <div className="i-ph:lightning text-sm" />
                Submit Task
              </Button>
            </div>
            <TxStatusIndicator status={txStatus} hash={txHash} />
          </CardContent>
        </Card>
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

function TxStatusIndicator({ status, hash }: { status: string; hash?: `0x${string}` }) {
  if (status === 'idle') return null;
  return (
    <div className={cn(
      'glass-card rounded-lg p-3',
      status === 'confirmed' && 'border-teal-500/30',
      status === 'failed' && 'border-crimson-500/30',
    )}>
      <div className="flex items-center gap-2">
        {status === 'signing' && <div className="i-ph:circle-fill text-xs text-amber-400 animate-pulse" />}
        {status === 'pending' && <div className="i-ph:circle-fill text-xs text-blue-400 animate-pulse" />}
        {status === 'confirmed' && <div className="i-ph:check-circle-fill text-xs text-teal-400" />}
        {status === 'failed' && <div className="i-ph:x-circle-fill text-xs text-crimson-400" />}
        <span className="text-xs text-cloud-elements-textSecondary">
          {status === 'signing' && 'Signing...'}
          {status === 'pending' && 'Confirming...'}
          {status === 'confirmed' && 'Confirmed'}
          {status === 'failed' && 'Failed'}
        </span>
        {hash && (
          <span className="text-xs font-data text-cloud-elements-textTertiary truncate max-w-[200px] ml-auto">
            {hash}
          </span>
        )}
      </div>
    </div>
  );
}
