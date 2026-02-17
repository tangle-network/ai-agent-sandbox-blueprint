import { Link } from 'react-router';
import { useStore } from '@nanostores/react';
import { AnimatedPage, StaggerContainer, StaggerItem } from '~/components/motion/AnimatedPage';
import { Card, CardContent } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { Badge } from '~/components/ui/badge';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { sandboxListStore, activeSandboxes } from '~/lib/stores/sandboxes';
import { cn } from '~/lib/utils';

export default function SandboxList() {
  const allSandboxes = useStore(sandboxListStore);
  const active = useStore(activeSandboxes);

  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">Sandboxes</h1>
          <p className="text-sm text-cloud-elements-textSecondary mt-1">
            {active.length > 0 ? `${active.length} active sandbox${active.length > 1 ? 'es' : ''}` : 'All your provisioned sandboxes'}
          </p>
        </div>
        <Link to="/create">
          <Button>
            <div className="i-ph:plus text-base" />
            New Sandbox
          </Button>
        </Link>
      </div>

      {allSandboxes.length > 0 ? (
        <StaggerContainer className="space-y-3">
          {allSandboxes.map((sb) => (
            <StaggerItem key={sb.id}>
              <Link to={`/sandboxes/${encodeURIComponent(sb.id)}`}>
                <Card className="hover:border-cloud-elements-borderColor transition-all">
                  <CardContent className="p-5">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-4 min-w-0">
                        <div className={cn(
                          'w-10 h-10 rounded-lg flex items-center justify-center shrink-0',
                          sb.status === 'running' ? 'bg-teal-500/10' :
                          sb.status === 'creating' ? 'bg-blue-500/10' :
                          sb.status === 'stopped' ? 'bg-amber-500/10' :
                          'bg-cloud-elements-background-depth-3',
                        )}>
                          <div className={cn(
                            'i-ph:hard-drives text-lg',
                            sb.status === 'running' ? 'text-teal-400' :
                            sb.status === 'creating' ? 'text-blue-400' :
                            sb.status === 'stopped' ? 'text-amber-400' :
                            'text-cloud-elements-textTertiary',
                          )} />
                        </div>
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <h3 className="text-sm font-display font-semibold text-cloud-elements-textPrimary truncate">{sb.name}</h3>
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
                      <div className="flex items-center gap-3">
                        <span className="text-xs font-data text-cloud-elements-textTertiary hidden md:block">
                          {new Date(sb.createdAt).toLocaleDateString()}
                        </span>
                        <div className="i-ph:caret-right text-lg text-cloud-elements-textTertiary" />
                      </div>
                    </div>
                  </CardContent>
                </Card>
              </Link>
            </StaggerItem>
          ))}
        </StaggerContainer>
      ) : (
        <Card>
          <CardContent className="p-6">
            <div className="py-16 text-center">
              <div className="i-ph:hard-drives text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-cloud-elements-textSecondary font-display">No sandboxes found</p>
              <p className="text-sm text-cloud-elements-textTertiary mt-1">Deploy a sandbox to see it here</p>
              <Link to="/create" className="inline-block mt-4">
                <Button variant="outline" size="sm">Create Sandbox</Button>
              </Link>
            </div>
          </CardContent>
        </Card>
      )}
    </AnimatedPage>
  );
}
