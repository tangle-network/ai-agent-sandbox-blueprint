import { Link } from 'react-router';
import { useStore } from '@nanostores/react';
import { AnimatedPage, StaggerContainer, StaggerItem } from '~/components/motion/AnimatedPage';
import { Card, CardContent } from '~/components/ui/card';
import { Button } from '~/components/ui/button';
import { StatusBadge } from '~/components/shared/StatusBadge';
import { instanceListStore, activeInstances } from '~/lib/stores/instances';
import { cn } from '~/lib/utils';

export default function InstanceList() {
  const allInstances = useStore(instanceListStore);
  const active = useStore(activeInstances);

  return (
    <AnimatedPage className="mx-auto max-w-7xl px-4 sm:px-6 py-8">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-display font-bold text-cloud-elements-textPrimary">Instances</h1>
          <p className="text-sm text-cloud-elements-textSecondary mt-1">
            {active.length > 0
              ? `${active.length} active instance${active.length > 1 ? 's' : ''}`
              : 'Subscription-based AI agent instances'}
          </p>
        </div>
        <Link to="/create?blueprint=ai-agent-instance-blueprint">
          <Button>
            <div className="i-ph:plus text-base" />
            New Instance
          </Button>
        </Link>
      </div>

      {allInstances.length > 0 ? (
        <StaggerContainer className="space-y-3">
          {allInstances.map((inst) => (
            <StaggerItem key={inst.id}>
              <Link to={`/instances/${encodeURIComponent(inst.id)}`}>
                <Card className="hover:border-cloud-elements-borderColor transition-all">
                  <CardContent className="p-5">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-4 min-w-0">
                        <div className={cn(
                          'w-10 h-10 rounded-lg flex items-center justify-center shrink-0',
                          inst.status === 'running' ? 'bg-blue-500/10' :
                          inst.status === 'creating' ? 'bg-violet-500/10' :
                          'bg-cloud-elements-background-depth-3',
                        )}>
                          <div className={cn(
                            inst.teeEnabled ? 'i-ph:shield-check' : 'i-ph:cube',
                            'text-lg',
                            inst.status === 'running' ? 'text-blue-400' :
                            inst.status === 'creating' ? 'text-violet-400' :
                            'text-cloud-elements-textTertiary',
                          )} />
                        </div>
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <h3 className="text-sm font-display font-semibold text-cloud-elements-textPrimary truncate">
                              {inst.name}
                            </h3>
                            <StatusBadge status={inst.status === 'creating' ? 'running' : inst.status} />
                            {inst.teeEnabled && (
                              <span className="text-xs text-violet-400 font-data">TEE</span>
                            )}
                          </div>
                          <div className="flex items-center gap-3 mt-1">
                            <span className="text-xs font-data text-cloud-elements-textTertiary">{inst.image}</span>
                            <span className="text-cloud-elements-dividerColor">·</span>
                            <span className="text-xs font-data text-cloud-elements-textTertiary">
                              {inst.cpuCores} CPU · {inst.memoryMb}MB
                            </span>
                          </div>
                        </div>
                      </div>
                      <div className="flex items-center gap-3">
                        <span className="text-xs font-data text-cloud-elements-textTertiary hidden md:block">
                          {new Date(inst.createdAt).toLocaleDateString()}
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
              <div className="i-ph:cube text-4xl text-cloud-elements-textTertiary mb-3 mx-auto" />
              <p className="text-cloud-elements-textSecondary font-display">No instances yet</p>
              <p className="text-sm text-cloud-elements-textTertiary mt-1">
                Provision an instance or TEE instance to get started
              </p>
              <Link to="/create?blueprint=ai-agent-instance-blueprint" className="inline-block mt-4">
                <Button variant="outline" size="sm">Create Instance</Button>
              </Link>
            </div>
          </CardContent>
        </Card>
      )}
    </AnimatedPage>
  );
}
