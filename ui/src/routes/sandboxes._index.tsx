import { useStore } from '@nanostores/react';
import { ProvisionedResourceListCard } from '~/components/shared/ProvisionedResourceListCard';
import { ProvisionedResourceIndexPage } from '~/components/shared/ProvisionedResourceIndexPage';
import { sandboxListStore, activeSandboxes } from '~/lib/stores/sandboxes';
import { useSandboxHydration } from '~/lib/hooks/useSandboxHydration';

export default function SandboxList() {
  useSandboxHydration();
  const allSandboxes = useStore(sandboxListStore);
  const active = useStore(activeSandboxes);

  return (
    <ProvisionedResourceIndexPage
      title="Sandboxes"
      subtitle={
        active.length > 0
          ? `${active.length} active sandbox${active.length > 1 ? 'es' : ''}`
          : 'All your provisioned sandboxes'
      }
      createTo="/create"
      createLabel="New Sandbox"
      items={allSandboxes}
      getKey={(sb) => sb.id}
      renderItem={(sb) => (
        <ProvisionedResourceListCard
          to={`/sandboxes/${encodeURIComponent(sb.id)}`}
          name={sb.name}
          status={sb.status}
          teeEnabled={sb.teeEnabled}
          image={sb.image}
          specs={`${sb.cpuCores} CPU · ${sb.memoryMb}MB · ${sb.diskGb}GB`}
          createdAt={sb.createdAt}
          iconClassName={sb.teeEnabled ? 'i-ph:shield-check' : 'i-ph:hard-drives'}
          iconContainerClassName={
            sb.status === 'running'
              ? 'bg-teal-500/10'
              : sb.status === 'creating'
                ? 'bg-blue-500/10'
                : sb.status === 'stopped'
                  ? 'bg-amber-500/10'
                  : 'bg-cloud-elements-background-depth-3'
          }
          iconToneClassName={
            sb.status === 'running'
              ? 'text-teal-400'
              : sb.status === 'creating'
                ? 'text-blue-400'
                : sb.status === 'stopped'
                  ? 'text-amber-400'
                  : 'text-cloud-elements-textTertiary'
          }
          teeStyle="pill"
        />
      )}
      emptyIconClassName="i-ph:hard-drives"
      emptyTitle="No sandboxes found"
      emptySubtitle="Deploy a sandbox to see it here"
      emptyCreateTo="/create"
      emptyCreateLabel="Create Sandbox"
    />
  );
}
