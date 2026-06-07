import { Link } from 'react-router';
import { useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { useAccount } from 'wagmi';
import { cn } from '@tangle-network/blueprint-ui';
import { Button } from '@tangle-network/blueprint-ui/components';
import { ConnectKitButton } from 'connectkit';
import {
  ConsoleChip,
  ConsoleMetricStrip,
  ConsolePage,
  ConsoleSection,
  EmptyConsoleState,
  type ConsoleMetric,
} from '~/components/console/ConsolePrimitives';
import {
  ResourceExplorerTable,
  type ResourceExplorerRow,
} from '~/components/console/ResourceExplorerTable';
import {
  sandboxListStore,
  runningSandboxes,
  stoppedSandboxes,
  getSandboxRouteKey,
  type LocalSandbox,
} from '~/lib/stores/sandboxes';
import {
  instanceListStore,
  runningInstances,
  type LocalInstance,
} from '~/lib/stores/instances';
import { useAvailableCapacity, useWorkflowIds } from '~/lib/hooks/useSandboxReads';

type ConsoleEvent = {
  key: string;
  label: string;
  detail: string;
  tone: 'brand' | 'ready' | 'warn' | 'danger' | 'muted';
  timestamp: number;
};

function formatNumber(value: unknown) {
  if (value == null) return '--';
  if (typeof value === 'bigint') return value.toString();
  if (typeof value === 'number') return Number.isFinite(value) ? String(value) : '--';
  return String(value);
}

function formatAge(timestamp: number | undefined) {
  if (!timestamp) return '--';
  const deltaMs = Date.now() - timestamp;
  if (deltaMs < 60_000) return '<1m ago';
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function getSecurityState(resource: Pick<LocalSandbox | LocalInstance, 'teeEnabled' | 'credentialsAvailable'>) {
  if (resource.teeEnabled) return 'attested';
  if (resource.credentialsAvailable) return 'secrets';
  return 'session';
}

function getStorageState(status: string) {
  if (status === 'warm' || status === 'cold') return status;
  if (status === 'stopped') return 'hot';
  if (status === 'gone') return 'gone';
  return 'ephemeral';
}

function sandboxToRow(sandbox: LocalSandbox): ResourceExplorerRow {
  return {
    key: sandbox.localId,
    href: `/sandboxes/${encodeURIComponent(getSandboxRouteKey(sandbox))}`,
    name: sandbox.name,
    id: sandbox.sandboxId ?? sandbox.localId,
    scope: 'Sandbox',
    status: sandbox.status,
    backend: sandbox.teeEnabled ? 'tee' : 'docker',
    image: sandbox.image,
    operator: sandbox.operator,
    specs: `${sandbox.cpuCores}c/${Math.round(sandbox.memoryMb / 1024)}g/${sandbox.diskGb}g`,
    sessions: sandbox.agentIdentifier ? 'agent' : '--',
    workflows: '--',
    network: sandbox.sshPort ? `ssh:${sandbox.sshPort}` : 'ports',
    security: getSecurityState(sandbox),
    storage: getStorageState(sandbox.status),
    createdAt: sandbox.createdAt,
    lastEvent: sandbox.lastActivityAt,
    teeEnabled: sandbox.teeEnabled,
    agentIdentifier: sandbox.agentIdentifier,
  };
}

function instanceToRow(instance: LocalInstance): ResourceExplorerRow {
  return {
    key: instance.id,
    href: `/instances/${encodeURIComponent(instance.id)}`,
    name: instance.name,
    id: instance.sandboxId ?? instance.id,
    scope: instance.teeEnabled ? 'TEE' : 'Instance',
    status: instance.status,
    backend: instance.teeEnabled ? 'tee' : 'docker',
    image: instance.image,
    operator: instance.operator,
    specs: `${instance.cpuCores}c/${Math.round(instance.memoryMb / 1024)}g/${instance.diskGb}g`,
    sessions: instance.agentIdentifier ? 'agent' : '--',
    workflows: '--',
    network: instance.sshPort ? `ssh:${instance.sshPort}` : 'ports',
    security: getSecurityState(instance),
    storage: getStorageState(instance.status),
    createdAt: instance.createdAt,
    teeEnabled: instance.teeEnabled,
    agentIdentifier: instance.agentIdentifier,
  };
}

function statusTone(status: string): ConsoleEvent['tone'] {
  if (status === 'running') return 'ready';
  if (status === 'creating') return 'brand';
  if (status === 'stopped' || status === 'warm' || status === 'cold') return 'warn';
  if (status === 'error') return 'danger';
  return 'muted';
}

function FirstRunLanding({ connected }: { connected: boolean }) {
  const { data: capacity } = useAvailableCapacity();
  const slots = typeof capacity === 'number' && Number.isFinite(capacity) ? capacity : null;

  return (
    <ConsolePage title="Tangle Sandbox Cloud" eyebrow="Tangle agent compute">
      <div className="mx-auto w-full max-w-4xl space-y-5">
        <div className="sandbox-console-panel rounded-[5px] p-6 lg:p-8">
          <h2 className="max-w-2xl font-display text-3xl font-bold leading-tight tracking-tight text-[var(--sandbox-console-text)]">
            Hardware-isolated sandboxes for AI agents
          </h2>
          <p className="mt-3 max-w-2xl text-[15px] leading-7 text-[var(--sandbox-console-muted)]">
            Rent compute from operators to run coding agents — Claude, Codex, opencode, Gemini, Kimi — in
            isolated sandboxes, or run your own node and provide capacity to the marketplace.
          </p>
          <div className="mt-5 flex flex-wrap items-center gap-2.5">
            {connected ? (
              <Link to="/create">
                <Button size="lg">
                  <span className="i-ph:rocket-launch text-base" />
                  Deploy a sandbox
                </Button>
              </Link>
            ) : (
              <ConnectKitButton.Custom>
                {({ show, isConnecting }) => (
                  <Button size="lg" onClick={show} disabled={isConnecting}>
                    <span className={cn('text-base', isConnecting ? 'i-ph:circle-notch animate-spin' : 'i-ph:plugs-connected')} />
                    {isConnecting ? 'Connecting' : 'Connect wallet to deploy'}
                  </Button>
                )}
              </ConnectKitButton.Custom>
            )}
            <Link to="/operators/register">
              <Button variant="secondary" size="lg">
                <span className="i-ph:hard-drives text-base" />
                Become an operator
              </Button>
            </Link>
          </div>
          {slots != null ? (
            <p className="mt-4 font-data text-[11px] uppercase tracking-[0.12em] text-[var(--sandbox-console-subtle)]">
              {slots} sandbox {slots === 1 ? 'slot' : 'slots'} available now across operators
            </p>
          ) : null}
        </div>

        <div className="grid gap-3 sm:grid-cols-3">
          {[
            {
              icon: 'i-ph:cube',
              title: 'Isolated by default',
              body: 'Each sandbox is a separate container or microVM with its own filesystem, network, and lifecycle.',
            },
            {
              icon: 'i-ph:terminal-window',
              title: 'Agent-native runtime',
              body: 'SSH, exec, prompt, and snapshot endpoints. Bring your harness; the node hosts the sidecar.',
            },
            {
              icon: 'i-ph:shield-check',
              title: 'Confidential option',
              body: 'TEE Instance operators run sandboxes inside Phala, Nitro, GCP, Azure, or direct TDX/SEV.',
            },
          ].map((cell) => (
            <div key={cell.title} className="sandbox-console-panel flex flex-col gap-2 rounded-[5px] p-4">
              <span className={cn('text-2xl text-[var(--sandbox-console-brand)]', cell.icon)} />
              <p className="font-display text-base font-bold text-[var(--sandbox-console-text)]">{cell.title}</p>
              <p className="text-[13px] leading-5 text-[var(--sandbox-console-muted)]">{cell.body}</p>
            </div>
          ))}
        </div>

        <div className="flex items-start gap-3 rounded-[5px] border border-amber-400/20 bg-amber-400/10 p-3.5">
          <span className="i-ph:info mt-0.5 text-base text-[var(--sandbox-console-warning)]" />
          <p className="text-[13px] leading-5 text-[var(--sandbox-console-muted)]">
            TEE attestation is collected and surfaced per resource, but is not yet cryptographically
            verified on-chain. Treat the TEE badge as operator-reported until verification ships.
          </p>
        </div>
      </div>
    </ConsolePage>
  );
}

export default function FleetConsole() {
  const { isConnected } = useAccount();
  const sandboxes = useStore(sandboxListStore);
  const running = useStore(runningSandboxes);
  const stopped = useStore(stoppedSandboxes);
  const instances = useStore(instanceListStore);
  const runningInst = useStore(runningInstances);
  const { data: capacity } = useAvailableCapacity();
  const { data: workflowIds } = useWorkflowIds(false);

  const hasResources = sandboxes.length > 0 || instances.length > 0;
  if (!isConnected || !hasResources) {
    return <FirstRunLanding connected={isConnected} />;
  }

  const resources = useMemo(
    () => [
      ...sandboxes.map(sandboxToRow),
      ...instances.map(instanceToRow),
    ].sort((left, right) => (right.lastEvent ?? right.createdAt) - (left.lastEvent ?? left.createdAt)),
    [instances, sandboxes],
  );

  const visibleResources = resources.slice(0, 8);
  const teeReady = resources.filter((resource) => resource.teeEnabled).length;
  const degraded = resources.filter((resource) => ['error', 'stopped', 'warm', 'cold'].includes(resource.status)).length;

  const metrics: ConsoleMetric[] = [
    {
      label: 'Running resources',
      value: String(running.length + runningInst.length),
      detail: `${resources.length} total`,
      tone: 'ready',
    },
    {
      label: 'Operator capacity',
      value: formatNumber(capacity),
      detail: 'available',
      tone: 'brand',
    },
    {
      label: 'Automation',
      value: workflowIds ? String(workflowIds.length) : '--',
      detail: 'registered',
      tone: 'warn',
    },
    {
      label: 'Security posture',
      value: String(teeReady),
      detail: degraded > 0 ? `${degraded} attention` : 'TEE resources',
      tone: degraded > 0 ? 'warn' : 'brand',
    },
  ];

  const events = useMemo<ConsoleEvent[]>(
    () => [
      ...sandboxes.map((sandbox) => ({
        key: `sandbox:${sandbox.localId}`,
        label: sandbox.status === 'running' ? 'Sandbox running' : `Sandbox ${sandbox.status}`,
        detail: sandbox.name,
        tone: statusTone(sandbox.status),
        timestamp: sandbox.lastActivityAt ?? sandbox.createdAt,
      })),
      ...instances.map((instance) => ({
        key: `instance:${instance.id}`,
        label: instance.status === 'running' ? 'Instance running' : `Instance ${instance.status}`,
        detail: instance.name,
        tone: statusTone(instance.status),
        timestamp: instance.createdAt,
      })),
    ].sort((left, right) => right.timestamp - left.timestamp).slice(0, 10),
    [instances, sandboxes],
  );

  return (
    <ConsolePage
      title="Fleet Console"
      eyebrow="Tangle agent compute"
      actions={(
        <Link to="/create">
          <Button>
            <span className="i-ph:plus text-base" />
            Launch
          </Button>
        </Link>
      )}
    >
      <div className="grid min-h-full gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div className="space-y-4">
          <ConsoleMetricStrip metrics={metrics} />

          <ConsoleSection title="Runtime Matrix">
            <div className="grid min-h-56 gap-px bg-[var(--sandbox-console-border)] p-px sm:grid-cols-2 xl:grid-cols-4">
              {[
                { label: 'cloud sandboxes', value: sandboxes.length, detail: `${running.length} running`, tone: 'ready' as const },
                { label: 'dedicated instances', value: instances.length, detail: `${runningInst.length} running`, tone: 'brand' as const },
                { label: 'stopped warm/cold', value: stopped.length, detail: 'resume candidates', tone: 'warn' as const },
                { label: 'degraded', value: resources.filter((resource) => resource.status === 'error').length, detail: 'errors', tone: 'danger' as const },
              ].map((cell) => (
                <div key={cell.label} className="flex min-h-40 flex-col justify-between bg-[var(--sandbox-console-panel)] p-4">
                  <p className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">
                    {cell.label}
                  </p>
                  <div>
                    <p className="font-data text-4xl font-semibold text-[var(--sandbox-console-text)]">{cell.value}</p>
                    <ConsoleChip tone={cell.tone}>{cell.detail}</ConsoleChip>
                  </div>
                </div>
              ))}
            </div>
          </ConsoleSection>

          <ConsoleSection title="Active Resource Ledger" actionTo="/sandboxes" actionLabel="Open Explorer">
            <ResourceExplorerTable
              rows={visibleResources}
              emptyTitle="No resources indexed"
              emptyDetail="Launch a sandbox or instance to populate the fleet ledger."
              emptyActionTo="/create"
              emptyActionLabel="Launch Resource"
            />
          </ConsoleSection>
        </div>

        <ConsoleSection title="Execution Tape" className="min-h-[420px]">
          {events.length > 0 ? (
            <div className="divide-y divide-[var(--sandbox-console-border)]">
              {events.map((event) => (
                <div key={event.key} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-3 py-3">
                  <span className="h-2 w-2 rounded-full bg-[var(--sandbox-console-muted)]" />
                  <div className="min-w-0">
                    <p className="truncate font-display text-sm font-medium text-[var(--sandbox-console-text)]">
                      {event.label}
                    </p>
                    <p className="truncate font-data text-[11px] text-[var(--sandbox-console-muted)]">
                      {event.detail}
                    </p>
                  </div>
                  <div className="text-right">
                    <ConsoleChip tone={event.tone}>{formatAge(event.timestamp)}</ConsoleChip>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <EmptyConsoleState
              icon="i-ph:pulse"
              title="No execution events"
              detail="Lifecycle, workflow, and terminal events appear here once resources exist."
            />
          )}
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}
