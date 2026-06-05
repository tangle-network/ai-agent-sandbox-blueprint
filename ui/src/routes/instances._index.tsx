import { Link } from 'react-router';
import { useMemo } from 'react';
import { useStore } from '@nanostores/react';
import { Button } from '@tangle-network/blueprint-ui/components';
import {
  ConsoleChip,
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
  activeInstances,
  instanceListStore,
  runningInstances,
  type LocalInstance,
} from '~/lib/stores/instances';
import { getInstanceStatusLabel } from '~/lib/instances/display';

function getSecurityState(instance: LocalInstance) {
  if (instance.teeEnabled) return 'attested';
  if (instance.credentialsAvailable) return 'secrets';
  return 'session';
}

function getStorageState(status: string) {
  if (status === 'stopped') return 'hot';
  if (status === 'gone') return 'gone';
  return 'ephemeral';
}

function instanceToRow(instance: LocalInstance): ResourceExplorerRow {
  return {
    key: instance.id,
    href: `/instances/${encodeURIComponent(instance.id)}`,
    name: instance.name,
    id: instance.sandboxId ?? instance.id,
    scope: instance.teeEnabled ? 'TEE' : 'Instance',
    status: instance.status,
    statusLabel: getInstanceStatusLabel(instance),
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

export default function InstanceExplorer() {
  const allInstances = useStore(instanceListStore);
  const active = useStore(activeInstances);
  const running = useStore(runningInstances);

  const rows = useMemo(
    () => allInstances.map(instanceToRow)
      .sort((left, right) => right.createdAt - left.createdAt),
    [allInstances],
  );

  const metrics: ConsoleMetric[] = [
    { label: 'Active', value: String(active.length), detail: 'dedicated', tone: 'ready' },
    { label: 'Running', value: String(running.length), detail: 'operator-backed', tone: 'ready' },
    { label: 'TEE instances', value: String(allInstances.filter((instance) => instance.teeEnabled).length), detail: 'sealed secrets', tone: 'brand' },
    { label: 'Errors', value: String(allInstances.filter((instance) => instance.status === 'error').length), detail: 'attention', tone: 'danger' },
  ];

  return (
    <ConsolePage
      title="Dedicated Instances"
      eyebrow="Singleton resources"
      actions={(
        <Link to="/create?blueprint=ai-agent-instance-blueprint">
          <Button>
            <span className="i-ph:plus text-base" />
            New Instance
          </Button>
        </Link>
      )}
    >
      <div className="space-y-4">
        <ConsoleMetricStrip metrics={metrics} />
        <ConsoleSection title="Instances">
          <ResourceExplorerTable
            rows={rows}
            emptyTitle="No instances indexed"
            emptyDetail="Provision a dedicated instance or TEE instance to inspect lifecycle, sessions, and trust state here."
            emptyActionTo="/create?blueprint=ai-agent-instance-blueprint"
            emptyActionLabel="Launch Instance"
          />
        </ConsoleSection>
        <div className="grid gap-3 md:grid-cols-3">
          <div className="sandbox-console-panel rounded-md p-3">
            <p className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">Modes</p>
            <div className="mt-3 flex flex-wrap gap-2">
              <ConsoleChip>instance</ConsoleChip>
              <ConsoleChip tone="brand">tee instance</ConsoleChip>
            </div>
          </div>
          <div className="sandbox-console-panel rounded-md p-3">
            <p className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">Lifecycle</p>
            <p className="mt-3 font-data text-xs text-[var(--sandbox-console-secondary)]">auto-provision · report provisioned · operator API</p>
          </div>
          <div className="sandbox-console-panel rounded-md p-3">
            <p className="font-data text-[10px] uppercase tracking-[0.14em] text-[var(--sandbox-console-muted)]">Isolation</p>
            <p className="mt-3 font-data text-xs text-[var(--sandbox-console-secondary)]">single tenant · optional attestation · sealed secrets</p>
          </div>
        </div>
      </div>
    </ConsolePage>
  );
}
