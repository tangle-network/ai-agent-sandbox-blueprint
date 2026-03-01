import { useStore } from '@nanostores/react';
import { ProvisionedResourceListCard } from '~/components/shared/ProvisionedResourceListCard';
import { ProvisionedResourceIndexPage } from '~/components/shared/ProvisionedResourceIndexPage';
import { instanceListStore, activeInstances } from '~/lib/stores/instances';

export default function InstanceList() {
  const allInstances = useStore(instanceListStore);
  const active = useStore(activeInstances);

  return (
    <ProvisionedResourceIndexPage
      title="Instances"
      subtitle={
        active.length > 0
          ? `${active.length} active instance${active.length > 1 ? 's' : ''}`
          : 'Subscription-based AI agent instances'
      }
      createTo="/create?blueprint=ai-agent-instance-blueprint"
      createLabel="New Instance"
      items={allInstances}
      getKey={(inst) => inst.id}
      renderItem={(inst) => (
        <ProvisionedResourceListCard
          to={`/instances/${encodeURIComponent(inst.id)}`}
          name={inst.name}
          status={inst.status}
          teeEnabled={inst.teeEnabled}
          image={inst.image}
          specs={`${inst.cpuCores} CPU · ${inst.memoryMb}MB`}
          createdAt={inst.createdAt}
          iconClassName={inst.teeEnabled ? 'i-ph:shield-check' : 'i-ph:cube'}
          iconContainerClassName={
            inst.status === 'running'
              ? 'bg-blue-500/10'
              : inst.status === 'creating'
                ? 'bg-violet-500/10'
                : 'bg-cloud-elements-background-depth-3'
          }
          iconToneClassName={
            inst.status === 'running'
              ? 'text-blue-400'
              : inst.status === 'creating'
                ? 'text-violet-400'
                : 'text-cloud-elements-textTertiary'
          }
          teeStyle="text"
        />
      )}
      emptyIconClassName="i-ph:cube"
      emptyTitle="No instances yet"
      emptySubtitle="Provision an instance or TEE instance to get started"
      emptyCreateTo="/create?blueprint=ai-agent-instance-blueprint"
      emptyCreateLabel="Create Instance"
    />
  );
}
