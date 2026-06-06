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
    { label: 'Instances', value: String(active.length), tone: 'ready' },
    { label: 'Running', value: String(running.length), tone: 'ready' },
    { label: 'TEE', value: String(allInstances.filter((instance) => instance.teeEnabled).length), tone: 'brand' },
    { label: 'Issues', value: String(allInstances.filter((instance) => instance.status === 'error').length), tone: 'danger' },
  ];

  return (
    <ConsolePage
      title="Instances"
      eyebrow="Cloud resources"
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
            emptyDetail="Create an instance to inspect runtime, sessions, network, and storage."
            emptyActionTo="/create?blueprint=ai-agent-instance-blueprint"
            emptyActionLabel="Launch Instance"
          />
        </ConsoleSection>
      </div>
    </ConsolePage>
  );
}
