import { Link } from 'react-router';
import { useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { Button } from '@tangle-network/blueprint-ui/components';
import {
  ConsoleMetricStrip,
  ConsolePage,
  ConsoleSection,
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

function getSecurityState(sandbox: LocalSandbox) {
  if (sandbox.teeEnabled) return 'attested';
  if (sandbox.credentialsAvailable) return 'secrets';
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

export default function SandboxExplorer() {
  const allSandboxes = useStore(sandboxListStore);
  const running = useStore(runningSandboxes);
  const stopped = useStore(stoppedSandboxes);

  const rows = useMemo(
    () => allSandboxes.map(sandboxToRow)
      .sort((left, right) => (right.lastEvent ?? right.createdAt) - (left.lastEvent ?? left.createdAt)),
    [allSandboxes],
  );

  const metrics: ConsoleMetric[] = [
    { label: 'Running', value: String(running.length), tone: 'ready' },
    { label: 'Paused', value: String(stopped.length), tone: 'warn' },
    { label: 'TEE', value: String(allSandboxes.filter((sandbox) => sandbox.teeEnabled).length), tone: 'brand' },
    { label: 'Issues', value: String(allSandboxes.filter((sandbox) => sandbox.status === 'error').length), tone: 'danger' },
  ];

  return (
    <ConsolePage
      title="Sandbox Explorer"
      eyebrow="Cloud fleet"
      actions={(
        <Link to="/create">
          <Button>
            <span className="i-ph:plus text-base" />
            New Sandbox
          </Button>
        </Link>
      )}
    >
      <div className="space-y-4">
        <ConsoleMetricStrip metrics={metrics} />
        <ConsoleSection title="Sandboxes">
          <ResourceExplorerTable
            rows={rows}
            emptyTitle="No sandboxes indexed"
            emptyDetail="Launch a sandbox to inspect runtime, sessions, network, security, and storage state here."
            emptyActionTo="/create"
            emptyActionLabel="Launch Sandbox"
          />
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}
